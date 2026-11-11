// SPDX-License-Identifier: MIT

use super::*;

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

/// A mount, the moved item, how it meets the destination, what happens to
/// its copy as the mount numbers it afresh, and whether the move deletes
/// its source.
struct Afresh {
    preset: Preset,
    item: &'static str,
    conflict: TransferConflict,
    change: Change,
    deleted: bool,
}

/// The paths of one row's move.
struct AfreshPaths {
    sources: PathBuf,
    destination: PathBuf,
    source: PathBuf,
    copy: PathBuf,
    notes: PathBuf,
    arrival: PathBuf,
    /// The copies the mount numbers afresh.
    copied: Vec<PathBuf>,
}

impl Afresh {
    fn row(&self) -> String {
        format!(
            "{:?}, {}, {:?}, {:?}",
            (self.preset)().mount_type,
            self.item,
            self.conflict,
            self.change
        )
    }

    fn fill(&self, root: &Path) -> Result<AfreshPaths, Box<dyn Error>> {
        let sources = root.join("sources");
        let destination = root.join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let source = sources.join(self.item);
        let copy = destination.join(self.item);
        let mut copied = vec![copy.clone()];
        let notes = if self.item == "folder" {
            fs::create_dir_all(source.join("sub"))?;
            fs::write(source.join("sub/b.txt"), b"b")?;
            copied.extend(["notes.txt", "sub", "sub/b.txt"].map(|path| copy.join(path)));
            source.join("notes.txt")
        } else {
            source.clone()
        };
        if self.conflict == TransferConflict::Merge {
            fs::create_dir(&copy)?;
            fs::write(copy.join("stays.txt"), b"stays")?;
        }
        fs::write(&notes, b"notes")?;
        let arrival = root.join("arrival");
        fs::write(&arrival, b"other")?;
        self.settle_times(&[&notes, &arrival], root, &copy)?;
        if self.change == Change::FolderEvicted {
            copied.truncate(1);
        }
        Ok(AfreshPaths {
            sources,
            destination,
            source,
            copy,
            notes,
            arrival,
            copied,
        })
    }

    /// Settles the times a change must show in, and waits until what
    /// replaces the copy is born in a later tick than what it stands in for.
    fn settle_times(
        &self,
        files: &[&Path; 2],
        root: &Path,
        copy: &Path,
    ) -> Result<(), Box<dyn Error>> {
        if matches!(self.change, Change::Rewritten | Change::ReplacedAlike) {
            let hour_before = an_hour_ago()?;
            for file in files {
                set_times(file, hour_before, 0)?;
            }
        }
        let probe = root.join("probe");
        match self.change {
            Change::ReplacedAlike => wait_for_a_later_birth_time(&probe, &fs::metadata(files[1])?),
            Change::FolderReplaced => wait_for_a_later_birth_time(&probe, &fs::metadata(copy)?),
            _ => {}
        }
        Ok(())
    }

    fn copied_notes(&self, copy: &Path) -> PathBuf {
        if self.item == "folder" {
            copy.join("notes.txt")
        } else {
            copy.to_path_buf()
        }
    }

    /// Changes the copy, then numbers every copy afresh.
    fn racer(&self, paths: &AfreshPaths, evicting: Arc<AtomicBool>) -> Racer {
        let change = self.change;
        let (replaced, merged) = (self.copied_notes(&paths.copy), paths.copy.clone());
        let (arrival, copied) = (paths.arrival.clone(), paths.copied.clone());
        Box::new(move || {
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
        })
    }
}

fn check_afresh(case: &Afresh) -> Result<(), Box<dyn Error>> {
    let row = case.row();
    let within_the_tick = case.change == Change::ReplacedWithinTheTick;
    // Before the kernel's coarse clock, which may run a tick behind.
    let tick = SystemTime::now() - Duration::from_secs(1);
    let _clock = within_the_tick.then(|| ClockGuard::set(tick));
    // Multigrain source times: only the destination can call for a
    // comparison of contents.
    let _kernel = KernelGuard::set(Some((6, 13, 0)));
    let coarse_clock = within_the_tick.then_some(tick);
    let root = tempfile::tempdir()?;
    let paths = case.fill(root.path())?;
    let evicted = Arc::new(AtomicBool::new(false));
    let racer = case.racer(&paths, evicted.clone());
    let placed_last = if case.conflict == TransferConflict::Merge {
        "sub"
    } else {
        case.item
    };
    let _sources = FsQuirks::install(
        &paths.sources,
        Quirks {
            coarse_clock,
            ..Quirks::ext4()
        },
    );
    let _destination = FsQuirks::install(
        &paths.destination,
        Quirks {
            rename_flags: Support::Supported,
            tmpfile: Support::Rejected(Errno::OPNOTSUPP),
            coarse_clock,
            after: vec![(Op::Rename, placed_last, 0, racer)],
            ..(case.preset)()
        },
    );

    let (_operation, events) = paste_move(98, &paths.destination, &paths.source, case.conflict);
    pump_until_transfer(&events);

    assert_pasted(&events, &row);
    assert!(evicted.load(Ordering::Acquire), "{row}");
    if case.deleted {
        assert!(
            !paths.source.exists(),
            "{row}: {:?}",
            reported(&events).kept
        );
        let copied_notes = case.copied_notes(&paths.copy);
        assert_eq!(fs::read(copied_notes)?, b"notes", "{row}");
    } else {
        assert_eq!(
            fs::read(&paths.notes).ok(),
            Some(b"notes".to_vec()),
            "{row}"
        );
        assert_eq!(
            reported(&events).kept,
            [kept(paths.source, KeptCause::CopyGone)],
            "{row}"
        );
    }
    Ok(())
}

#[test]
fn copies_a_mount_numbers_afresh_are_known_by_birth_time_or_else_by_inode_number()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // exFAT and FUSE daemons without `use_ino` number an entry afresh once
    // the kernel evicts it, here right after the copy is published; on ext4,
    // which keeps inode numbers, another number is another file. Merging into
    // a folder on exFAT sets its birth time again. Whether the move deletes
    // its source.
    let fail = |preset, item, change, deleted| Afresh {
        preset,
        item,
        conflict: TransferConflict::FailIfExists,
        change,
        deleted,
    };
    let merge = |preset, change, deleted| Afresh {
        conflict: TransferConflict::Merge,
        ..fail(preset, "folder", change, deleted)
    };
    let rows = [
        fail(Quirks::sshfs, "notes.txt", Change::None, false),
        fail(Quirks::sshfs, "folder", Change::FolderEvicted, false),
        fail(Quirks::exfat_fuse, "folder", Change::FolderEvicted, false),
        fail(Quirks::vfat, "folder", Change::None, true),
        fail(Quirks::exfat, "folder", Change::None, true),
        fail(Quirks::exfat, "folder", Change::Rewritten, false),
        fail(
            Quirks::exfat_fuse,
            "folder",
            Change::ReplacedWithinTheTick,
            false,
        ),
        fail(Quirks::ext4, "notes.txt", Change::None, false),
        merge(Quirks::exfat, Change::None, true),
        merge(Quirks::exfat, Change::ReplacedAlike, false),
        merge(Quirks::vfat, Change::FolderReplaced, false),
    ];
    for case in &rows {
        check_afresh(case)?;
    }
    Ok(())
}

/// A mount, the moved item, how it meets the destination, and whether
/// another link takes the placed link's place.
struct LinkAfresh {
    preset: Preset,
    item: &'static str,
    conflict: TransferConflict,
    replaced: bool,
}

impl LinkAfresh {
    /// Without `RENAME_NOREPLACE`, the link is made again at its name.
    fn made_again(&self) -> bool {
        matches!((self.preset)().rename_flags, Support::Rejected(_))
    }

    fn row(&self) -> String {
        format!(
            "{:?}, {}, {:?}, made again: {}, replaced: {}",
            (self.preset)().mount_type,
            self.item,
            self.conflict,
            self.made_again(),
            self.replaced
        )
    }

    /// The call after which the link stands at `placed`; a link in a
    /// staged folder is first made there under its name.
    fn placing(&self, placed: &'static str, racer: Racer) -> Before {
        if self.made_again() {
            (
                Op::Symlink,
                placed,
                usize::from(self.item == "folder"),
                racer,
            )
        } else {
            (Op::Rename, placed, 0, racer)
        }
    }
}

/// Replaces the placed link with `arrival` if asked, then numbers it afresh.
fn renumbering_link(
    replaced: bool,
    arrival: PathBuf,
    copied: PathBuf,
    evicting: Arc<AtomicBool>,
) -> Racer {
    Box::new(move || {
        if replaced {
            fs::rename(&arrival, &copied).expect("replace the link");
        }
        super::super::fs_quirks::renumbered(&copied);
        evicting.store(true, Ordering::Release);
    })
}

fn check_link_afresh(case: &LinkAfresh) -> Result<(), Box<dyn Error>> {
    let row = case.row();
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir(&sources)?;
    fs::create_dir(&destination)?;
    let source = sources.join(case.item);
    let copy = destination.join(case.item);
    let (link, copied) = if case.item == "folder" {
        fs::create_dir(&source)?;
        (source.join("link"), copy.join("link"))
    } else {
        (source.clone(), copy.clone())
    };
    let placed = if case.conflict == TransferConflict::Merge {
        fs::create_dir(&copy)?;
        "link"
    } else {
        case.item
    };
    std::os::unix::fs::symlink("original-target", &link)?;
    let arrival = root.path().join("arrival");
    std::os::unix::fs::symlink("another-target", &arrival)?;
    let evicted = Arc::new(AtomicBool::new(false));
    let racer = renumbering_link(case.replaced, arrival, copied.clone(), evicted.clone());
    let _sources = FsQuirks::install(&sources, Quirks::ext4());
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            tmpfile: Support::Rejected(Errno::OPNOTSUPP),
            after: vec![case.placing(placed, racer)],
            ..(case.preset)()
        },
    );

    let (_operation, events) = paste_move(98, &destination, &source, case.conflict);
    pump_until_transfer(&events);

    assert!(evicted.load(Ordering::Acquire), "{row}");
    let original = Some(PathBuf::from("original-target"));
    if case.replaced {
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
        check_link_afresh(&LinkAfresh {
            preset,
            item,
            conflict,
            replaced,
        })?;
    }
    Ok(())
}
