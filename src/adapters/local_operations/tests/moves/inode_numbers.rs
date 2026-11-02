// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, PartialEq)]
enum Linked {
    No,
    Outside,
    SelectedFirst,
}

#[derive(Clone, Copy)]
enum Meanwhile {
    Renumbered,
    ReplacedAlike { linked: Linked },
}

type MakeQuirks = Box<dyn Fn() -> Quirks>;

fn vfat_flushed() -> Quirks {
    Quirks {
        mount_type: Some(("vfat", "rw,flush")),
        ..Quirks::default()
    }
}

fn share(preset: Preset, no_birth_time: bool) -> MakeQuirks {
    Box::new(move || Quirks {
        mount_type: preset().mount_type,
        no_birth_time,
        ..Quirks::default()
    })
}

/// A source mount, whether its server inode numbers are turned off during
/// the move, what happens to the moved folder meanwhile, and what the move
/// keeps at the source.
struct Numbers {
    case: &'static str,
    quirks: MakeQuirks,
    turned_off: bool,
    meanwhile: Meanwhile,
    kept_at_source: Option<&'static str>,
}

fn renumbered(
    case: &'static str,
    quirks: MakeQuirks,
    turned_off: bool,
    kept_at_source: Option<&'static str>,
) -> Numbers {
    Numbers {
        case,
        quirks,
        turned_off,
        meanwhile: Meanwhile::Renumbered,
        kept_at_source,
    }
}

/// A file of the moved folder replaced on cifs by one alike.
fn replaced(case: &'static str, turned_off: bool, linked: Linked) -> Numbers {
    Numbers {
        case,
        quirks: share(Quirks::cifs, false),
        turned_off,
        meanwhile: Meanwhile::ReplacedAlike { linked },
        kept_at_source: Some("folder/a.txt"),
    }
}

/// The moved `folder` beside `backup.txt` in `source`, and the `arrival`
/// that may replace its `a.txt`.
struct NumberedTree {
    root: PathBuf,
    source: PathBuf,
    folder: PathBuf,
    arrival: PathBuf,
    backup: PathBuf,
}

impl NumberedTree {
    fn create(root: &Path) -> io::Result<Self> {
        let source = root.join("source");
        let folder = source.join("folder");
        fs::create_dir_all(folder.join("sub"))?;
        fs::write(folder.join("a.txt"), b"a")?;
        fs::write(folder.join("sub/b.txt"), b"b")?;
        Ok(Self {
            root: root.to_path_buf(),
            arrival: root.join("arrival"),
            backup: source.join("backup.txt"),
            source,
            folder,
        })
    }

    fn evicted(&self) -> [PathBuf; 4] {
        let folder = &self.folder;
        [
            folder.clone(),
            folder.join("a.txt"),
            folder.join("sub"),
            folder.join("sub/b.txt"),
        ]
    }
}

impl Numbers {
    fn selected_first(&self) -> bool {
        matches!(
            self.meanwhile,
            Meanwhile::ReplacedAlike {
                linked: Linked::SelectedFirst
            }
        )
    }

    /// Gives `a.txt` and the `arrival` alike settled times, or says why this
    /// base cannot run the case.
    fn prepare_replacement(
        &self,
        tree: &NumberedTree,
    ) -> Result<Result<(), &'static str>, Box<dyn Error>> {
        let Meanwhile::ReplacedAlike { linked } = self.meanwhile else {
            return Ok(Ok(()));
        };
        let file = tree.folder.join("a.txt");
        if linked != Linked::No && !hard_linked(&file, &tree.backup)? {
            return Ok(Err("a base without hard links"));
        }
        fs::write(&tree.arrival, b"z")?;
        let hour_before = an_hour_ago()?;
        set_times(&file, hour_before, 123_456_789)?;
        if self.turned_off && !a_later_change_time(&tree.arrival, &file, hour_before)? {
            return Ok(Err(
                "a base that reports the modification time as the change time",
            ));
        }
        set_times(&tree.arrival, hour_before, 123_456_789)?;
        Ok(Ok(()))
    }

    /// Once the server inode numbers go, renumbers the folder or replaces
    /// its file, recording whether that happened.
    fn once_numbers_go(&self, tree: &NumberedTree, switched: Arc<AtomicBool>) -> Racer {
        let (turned_off, meanwhile, evicted) = (self.turned_off, self.meanwhile, tree.evicted());
        let at = tree.source.clone();
        let (probe, file, arrival) = (
            tree.root.join("probe"),
            tree.folder.join("a.txt"),
            tree.arrival.clone(),
        );
        Box::new(move || {
            let numbers_went = !turned_off || super::super::fs_quirks::autodisable_serverino(&at);
            let happened = numbers_went
                && match meanwhile {
                    Meanwhile::Renumbered => evicted
                        .iter()
                        .all(|path| super::super::fs_quirks::evict(path)),
                    Meanwhile::ReplacedAlike { .. } => {
                        let before = fs::metadata(&file).expect("stat the file");
                        wait_for_a_later_change_time(&probe, &before);
                        fs::rename(&arrival, &file).is_ok()
                    }
                };
            switching(&switched, happened);
        })
    }

    /// The racers before calls at the source, after calls at the source,
    /// and after calls at the destination.
    fn racers(
        &self,
        tree: &NumberedTree,
        switched: &Arc<AtomicBool>,
    ) -> (Vec<Before>, Vec<Before>, Vec<Before>) {
        if let (Meanwhile::Renumbered, false) = (self.meanwhile, self.turned_off) {
            let (evicted, switched) = (tree.evicted(), switched.clone());
            let racer: Racer = Box::new(move || {
                for path in &evicted {
                    super::super::fs_quirks::renumbered(path);
                }
                switching(&switched, true);
            });
            return (
                vec![(Op::OpenDir, "folder", 1, racer)],
                Vec::new(),
                Vec::new(),
            );
        }
        let racer = self.once_numbers_go(tree, switched.clone());
        if self.selected_first() {
            (
                Vec::new(),
                vec![(Op::Unlink, "backup.txt", 0, racer)],
                Vec::new(),
            )
        } else {
            (
                Vec::new(),
                Vec::new(),
                vec![(Op::Rename, "folder", 0, racer)],
            )
        }
    }
}

fn switching(switched: &AtomicBool, happened: bool) {
    switched.store(happened, Ordering::Release);
}

fn check_numbers(numbers: &Numbers) -> Result<(), Box<dyn Error>> {
    let case = numbers.case;
    let root = tempfile::tempdir()?;
    let tree = NumberedTree::create(root.path())?;
    if let Err(base) = numbers.prepare_replacement(&tree)? {
        eprintln!("skipped on {base}: {case}");
        return Ok(());
    }
    let switched = Arc::new(AtomicBool::new(false));
    let (before, after_at_source, after_at_destination) = numbers.racers(&tree, &switched);
    let _source = FsQuirks::install(
        &tree.source,
        Quirks {
            before,
            after: after_at_source,
            ..(numbers.quirks)()
        },
    );
    let destination = root.path().join("destination");
    fs::create_dir_all(&destination)?;
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            after: after_at_destination,
            ..Quirks::default()
        },
    );

    let sources = if numbers.selected_first() {
        vec![tree.backup.as_path(), &tree.folder]
    } else {
        vec![tree.folder.as_path()]
    };
    let (_operation, events) =
        paste_moves(95, &destination, &sources, TransferConflict::FailIfExists);
    pump_until_transfer(&events);

    assert_pasted(&events, case);
    assert!(switched.load(Ordering::Acquire), "{case}");
    if numbers.selected_first() {
        assert_eq!(fs::read(destination.join("backup.txt"))?, b"a", "{case}");
        assert!(!tree.backup.exists(), "{case}");
    }
    assert_files(
        &destination,
        &[("folder/a.txt", "a"), ("folder/sub/b.txt", "b")],
        case,
    )?;
    assert_kept_at_source(numbers, &tree, &reported(&events))?;
    Ok(())
}

fn assert_kept_at_source(
    numbers: &Numbers,
    tree: &NumberedTree,
    report: &TransferReport,
) -> io::Result<()> {
    let case = numbers.case;
    if let Meanwhile::ReplacedAlike { .. } = numbers.meanwhile {
        let left = fs::read(tree.folder.join("a.txt")).ok();
        assert_eq!(left, Some(b"z".to_vec()), "{case}");
    }
    let kept_at_source = numbers.kept_at_source;
    assert_eq!(tree.folder.exists(), kept_at_source.is_some(), "{case}");
    let expected: Vec<_> = kept_at_source
        .map(|path| kept(tree.source.join(path), KeptCause::Changed))
        .into_iter()
        .collect();
    assert_eq!(report.kept, expected, "{case}");
    Ok(())
}

#[test]
fn moves_compare_inode_numbers_only_where_the_source_keeps_them() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let rows = [
        renumbered("sshfs", Box::new(Quirks::sshfs), false, None),
        renumbered(
            "cifs noserverino",
            share(Quirks::cifs_noserverino, false),
            false,
            None,
        ),
        renumbered("cifs", share(Quirks::cifs, false), false, Some("folder")),
        renumbered(
            "cifs without creation times",
            share(Quirks::cifs, true),
            false,
            Some("folder"),
        ),
        renumbered(
            "cifs, server inode numbers turned off",
            share(Quirks::cifs, false),
            true,
            None,
        ),
        renumbered(
            "cifs without creation times, server inode numbers turned off",
            share(Quirks::cifs, true),
            true,
            None,
        ),
        renumbered("vfat", Box::new(vfat_flushed), false, None),
        replaced("cifs, a linked file replaced", false, Linked::Outside),
        replaced(
            "cifs, a file replaced once server inode numbers went",
            true,
            Linked::No,
        ),
        replaced(
            "cifs, a linked file replaced once server inode numbers went",
            true,
            Linked::Outside,
        ),
        replaced(
            "cifs, a file linked to an item moved first replaced",
            false,
            Linked::SelectedFirst,
        ),
        replaced(
            "cifs, a file linked to an item moved first replaced once server inode numbers went",
            true,
            Linked::SelectedFirst,
        ),
    ];
    for numbers in &rows {
        check_numbers(numbers)?;
    }
    Ok(())
}
