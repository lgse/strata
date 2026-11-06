// SPDX-License-Identifier: MIT

use std::sync::atomic::AtomicUsize;

use super::*;

fn check_changed_or_added(conflict: TransferConflict) -> Result<(), Box<dyn Error>> {
    let row = format!("{conflict:?}");
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
    let racer: Racer = Box::new(move || {
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

    assert_pasted(&events, &row);
    let moved = destination.join("folder");
    assert_files(&moved, &[("a.txt", "a"), ("sub/b.txt", "b")], &row)?;
    assert_eq!(names(&folder)?, ["added.txt", "sub"], "{row}");
    assert_files(
        &folder,
        &[("sub/b.txt", "rewritten"), ("added.txt", "added")],
        &row,
    )?;
    assert_eq!(
        reported(&events).kept,
        [
            kept(changed, KeptCause::Changed),
            kept(folder.clone(), KeptCause::Changed)
        ],
        "{row}"
    );
    Ok(())
}

#[test]
fn a_move_keeps_source_entries_changed_or_added_after_their_copy() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for conflict in MOVES {
        check_changed_or_added(conflict)?;
    }
    Ok(())
}

/// A file `db` rewritten as its own move reads it: in a moved folder, with
/// another name in it, or selected itself.
struct WrittenDuring {
    kind: &'static str,
    rewrites: usize,
    destination_quirks: Preset,
}

impl WrittenDuring {
    fn row(&self) -> String {
        format!("{}, rewrites: {}", self.kind, self.rewrites)
    }

    /// Writes the folder, and returns which names the reads are watched on.
    fn fill(&self, folder: &Path, db: &Path) -> io::Result<&'static str> {
        fs::create_dir_all(folder)?;
        fs::write(db, b"first")?;
        if self.kind == "linked" {
            fs::hard_link(db, folder.join("DB"))?;
            return Ok("*");
        }
        fs::write(folder.join("notes.txt"), b"notes")?;
        Ok("db")
    }

    /// Each pass reads the data, then the end of the file.
    fn rewrites(&self, reads: &'static str, db: &Path) -> Vec<Before> {
        let mut before = vec![(Op::Read, reads, 0, rewrite(db, b"second write"))];
        if self.rewrites == 2 {
            before.push((Op::Read, reads, 1, rewrite(db, b"third")));
        }
        before
    }
}

fn check_written_during(case: &WrittenDuring) -> Result<(), Box<dyn Error>> {
    let row = case.row();
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let folder = source_dir.join("folder");
    let db = folder.join("db");
    let reads = case.fill(&folder, &db)?;
    let _source = FsQuirks::install(
        &source_dir,
        Quirks {
            before: case.rewrites(reads, &db),
            ..Quirks::default()
        },
    );
    let destination = root.path().join("destination");
    fs::create_dir_all(&destination)?;
    let _destination = FsQuirks::install(&destination, (case.destination_quirks)());
    let moved = if case.kind == "selected" {
        &db
    } else {
        &folder
    };

    let (_operation, events) = paste_move(93, &destination, moved, TransferConflict::FailIfExists);
    pump_until_transfer(&events);

    if case.kind == "selected" {
        return assert_selected_refused(&events, &destination, &db, &row);
    }
    assert_pasted(&events, &row);
    let left = kept_in_order(reported(&events).kept);
    let moved = destination.join("folder");
    if case.rewrites == 1 {
        assert_eq!(fs::read(moved.join("db"))?, b"second write", "{row}");
        assert!(!folder.exists(), "{row}");
        assert!(left.is_empty(), "{row}");
        return Ok(());
    }
    let (copied, stayed, changed): (&[&str], &[&str], &[&str]) = if case.kind == "linked" {
        (&[], &["DB", "db"], &["DB", "db"])
    } else {
        (&["notes.txt"], &["db"], &["db"])
    };
    assert_eq!(names(&moved)?, copied, "{row}");
    assert_eq!(names(&folder)?, stayed, "{row}");
    assert_eq!(fs::read(&db)?, b"third", "{row}");
    let changed: Vec<_> = changed
        .iter()
        .map(|name| kept(folder.join(name), KeptCause::Changed))
        .collect();
    assert_eq!(left, changed, "{row}");
    Ok(())
}

/// A selected file rewritten during its copy fails the paste and stays.
fn assert_selected_refused(
    events: &RefCell<Vec<OperationEvent>>,
    destination: &Path,
    db: &Path,
    row: &str,
) -> Result<(), Box<dyn Error>> {
    let terminal = terminal_transfer(&events.borrow()).cloned();
    let Some(OperationEvent::TransferFailed { message, .. }) = terminal else {
        panic!("{row}: {:?}", events.borrow());
    };
    assert_eq!(
        message,
        rust_i18n::t!("%{name} changed while it was being copied", name = "db"),
        "{row}"
    );
    assert!(names(destination)?.is_empty(), "{row}");
    assert_eq!(fs::read(db)?, b"third", "{row}");
    Ok(())
}

#[test]
fn a_file_written_during_its_own_move_stays_at_the_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let case = |kind, rewrites, destination_quirks: Preset| WrittenDuring {
        kind,
        rewrites,
        destination_quirks,
    };
    let cases = [
        case("in a folder", 1, Quirks::default),
        case("in a folder", 2, Quirks::default),
        case("linked", 2, Quirks::nfs3),
        case("selected", 2, Quirks::default),
    ];
    for case in &cases {
        check_written_during(case)?;
    }
    Ok(())
}

/// The second name of a linked pair is rewritten once the deletion is
/// under way, with the first name's times put back.
struct LinkedRewrite {
    deleting: Arc<AtomicBool>,
    checked: Arc<AtomicUsize>,
    went: Arc<AtomicBool>,
}

impl LinkedRewrite {
    fn new(numbers_go: bool) -> Self {
        Self {
            deleting: Arc::new(AtomicBool::new(false)),
            checked: Arc::new(AtomicUsize::new(0)),
            went: Arc::new(AtomicBool::new(!numbers_go)),
        }
    }

    fn rewrite_second(&self, path: PathBuf, probe: PathBuf, hour_before: u64) -> Racer {
        let (deleting, checked) = (self.deleting.clone(), self.checked.clone());
        Box::new(move || {
            let second_check = || checked.fetch_add(1, Ordering::SeqCst) == 1;
            if !(deleting.load(Ordering::Acquire) && second_check()) {
                return;
            }
            let before = fs::metadata(&path).expect("stat the file");
            wait_for_a_later_change_time(&probe, &before);
            fs::write(&path, b"LINKED").expect("rewrite the file");
            set_times(&path, hour_before, 0).expect("put the times back");
        })
    }

    /// As the deletion opens the folder, its names may be numbered afresh.
    fn deletion_opens(&self, linked: [PathBuf; 2], evicted: bool) -> Racer {
        let (opening, renumbering) = (self.deleting.clone(), self.went.clone());
        Box::new(move || {
            if evicted {
                renumbering.fetch_and(
                    linked
                        .iter()
                        .all(|path| super::super::fs_quirks::evict(path)),
                    Ordering::AcqRel,
                );
            }
            opening.store(true, Ordering::Release);
        })
    }

    fn numbers_went(&self, at: PathBuf) -> Racer {
        let going = self.went.clone();
        Box::new(move || {
            going.store(
                super::super::fs_quirks::autodisable_serverino(&at),
                Ordering::Release,
            );
        })
    }
}

/// Makes `folder` hold a linked pair with settled times, or says why this
/// base cannot run the row.
fn linked_pair(
    folder: &Path,
    numbers_go: bool,
) -> Result<Result<u64, &'static str>, Box<dyn Error>> {
    fs::create_dir_all(folder)?;
    fs::write(folder.join("a"), b"linked")?;
    if !hard_linked(&folder.join("a"), &folder.join("b"))? {
        return Ok(Err("a base without hard links"));
    }
    if !numbers_go && super::super::fs_quirks::smb_base(fs::File::open(folder)?.as_fd()) {
        return Ok(Err(
            "an SMB base, which refuses to open a link's second name while the first is open",
        ));
    }
    let hour_before = an_hour_ago()?;
    set_times(&folder.join("a"), hour_before, 0)?;
    if numbers_go && !a_later_change_time(&folder.join("b"), &folder.join("a"), hour_before)? {
        return Ok(Err(
            "a base that reports the modification time as the change time",
        ));
    }
    Ok(Ok(hour_before))
}

fn check_linked_name_changed(
    row: &str,
    source: Preset,
    numbers_go: bool,
    evicted: bool,
) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let folder = source_dir.join("folder");
    let hour_before = match linked_pair(&folder, numbers_go)? {
        Ok(hour_before) => hour_before,
        Err(base) => {
            eprintln!("skipped on {base}: {row}");
            return Ok(());
        }
    };
    let rewrite = LinkedRewrite::new(numbers_go);
    let linked = [folder.join("a"), folder.join("b")];
    let mut before = vec![(
        Op::OpenDir,
        "folder",
        1,
        rewrite.deletion_opens(linked, evicted),
    )];
    if numbers_go {
        before.push((
            Op::Stat,
            "folder",
            0,
            rewrite.numbers_went(source_dir.clone()),
        ));
    }
    for name in ["a", "b", "a", "b"] {
        let probe = root.path().join("probe");
        let racer = rewrite.rewrite_second(folder.join(name), probe, hour_before);
        before.push((Op::Stat, name, 0, racer));
    }
    let quirks = Quirks { before, ..source() };

    let (moved, report, _) = move_across_volumes(&folder, quirks)?;

    assert!(rewrite.went.load(Ordering::Acquire), "{row}");
    assert_files(&moved, &[("a", "linked"), ("b", "linked")], row)?;
    assert!(folder.exists(), "{row}");
    let left = names(&folder)?;
    assert_eq!(left.len(), 1, "{row}: {left:?}");
    let second = folder.join(&left[0]);
    assert_eq!(fs::read(&second)?, b"LINKED", "{row}");
    assert_eq!(report.kept, [kept(second, KeptCause::Changed)], "{row}");
    Ok(())
}

#[test]
fn a_name_of_a_linked_file_changed_after_its_other_name_went_stays() -> Result<(), Box<dyn Error>> {
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
        check_linked_name_changed(row, source, numbers_go, evicted)?;
    }
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
        let racer: Racer = Box::new(move || {
            let before = fs::metadata(&path).expect("stat the source");
            wait_for_a_later_change_time(&probe, &before);
            fs::write(&path, b"B2").expect("rewrite the source");
            put_times_back(&path, &before).expect("put the times back");
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

/// What a racer does to the moved folder once its copy is done.
fn swapping(row: &str, at: PathBuf) -> (Vec<Before>, Vec<Before>) {
    match row {
        "swapped" => {
            let racer: Racer = Box::new(move || {
                fs::remove_dir_all(at.join("sub")).expect("remove the folder");
                fs::write(at.join("sub"), b"a file now").expect("put a file there");
            });
            (vec![(Op::OpenDir, "sub", 1, racer)], Vec::new())
        }
        "removed" => {
            let racer: Racer = Box::new(move || {
                fs::remove_file(at.join("a.txt")).expect("remove the file");
            });
            (vec![(Op::OpenDir, "folder", 1, racer)], Vec::new())
        }
        _ => {
            let racer: Racer = Box::new(move || {
                fs::rename(&at, at.with_file_name("emptied")).expect("move it");
                fs::create_dir(&at).expect("make another folder");
            });
            (Vec::new(), vec![(Op::Unlink, "a.txt", 0, racer)])
        }
    }
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
        let (before, after) = swapping(row, folder.clone());
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
