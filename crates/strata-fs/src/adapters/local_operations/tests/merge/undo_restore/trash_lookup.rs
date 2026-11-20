// SPDX-License-Identifier: MIT

use super::*;

/// Looks originals up among the entries of the one Trash at `trash_root`
/// by their recorded identities.
fn recorded_identity_lookup(
    trash_root: PathBuf,
    originals: HashMap<Location, ItemIdentity>,
) -> StagedOriginalLookup {
    Rc::new(move |location, cancellable| {
        let path = location.native_path().expect("local original").to_owned();
        let (trash_root, originals) = (trash_root.clone(), originals.clone());
        Box::pin(async move {
            gio::spawn_blocking(move || {
                trash_entries_at(
                    std::slice::from_ref(&trash_root),
                    &HashSet::from([path.clone()]),
                    &cancellable,
                    &originals,
                )
                .remove(&path)
            })
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| Err(io_error("Recorded original is missing")))
        })
    })
}

const COLLIDING: [&str; 3] = ["older.zip", "current.zip", "purged.zip"];

fn check_recorded_identity(chosen: &str) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let target = root.path().join("archive.zip");
    fs::write(&target, b"incoming archive")?;
    let mut staged = HashMap::new();
    for name in COLLIDING {
        let entry = staged_trash_fixture(root.path(), name, &target, name.as_bytes())?;
        staged.insert(
            name,
            entry.source.native_path().expect("staged path").to_owned(),
        );
    }
    let original = identity_off_main(&staged[chosen]).ok_or("identity")?;
    let purged = chosen == "purged.zip";
    if purged {
        fs::remove_file(&staged[chosen])?;
    }
    let trash_root = staged[chosen]
        .parent()
        .expect("trash files directory")
        .parent()
        .expect("trash root")
        .to_owned();
    let location = Location::local(&target);
    let originals = HashMap::from([(location.clone(), original)]);
    let lookup = recorded_identity_lookup(trash_root, originals);
    let placed = placed_now(&[&target]);
    let events = undo_merge(
        91,
        (Vec::new(), vec![location], placed),
        gio::Cancellable::new(),
        lookup,
    );

    let (expected, ended): (&[u8], bool) = if purged {
        let ended = matches!(
            events.last(),
            Some(OperationEvent::CompletedWithErrors { .. })
        );
        (b"incoming archive", ended)
    } else {
        let ended = matches!(events.last(), Some(OperationEvent::Restored { .. }));
        (chosen.as_bytes(), ended)
    };
    assert_eq!(fs::read(&target)?, expected);
    assert!(ended);
    for (name, path) in staged.iter().filter(|(name, _)| **name != chosen) {
        assert_eq!(fs::read(path)?, name.as_bytes());
    }
    Ok(())
}

#[test]
fn merge_undo_uses_recorded_identity_when_trash_dates_collide() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for chosen in COLLIDING {
        check_recorded_identity(chosen)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum InTrash {
    Renumbered,
    NumberReused,
    TrashedAfter,
    TrashedInTheSameTick(&'static [u8]),
}

#[derive(Clone, Copy, Debug)]
enum Chosen {
    Item(&'static str),
    Nothing,
    Refused,
}

/// The per-user Trash beneath a mount, and the original `report` it holds
/// entries for, made as a file or a `folder`.
struct TrashRoom {
    trash_root: PathBuf,
    original: PathBuf,
    folder: bool,
}

impl TrashRoom {
    fn make(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        if self.folder {
            fs::create_dir(path)
        } else {
            fs::write(path, contents)
        }
    }

    fn info(&self, name: &str, date: &str) -> io::Result<()> {
        fs::write(
            self.trash_root
                .join("info")
                .join(format!("{name}.trashinfo")),
            format!(
                "[Trash Info]\nPath={}\nDeletionDate={date}T00:00:00\n",
                self.original.display()
            ),
        )
    }

    fn put_in_trash(&self, name: &str, contents: &[u8], date: &str) -> io::Result<()> {
        self.make(&self.trash_root.join("files").join(name), contents)?;
        self.info(name, date)
    }

    /// Changes what Trash holds after the original went in as `trashed`.
    fn change(
        &self,
        change: InTrash,
        trashed: &Path,
        modified: SystemTime,
    ) -> Result<(), Box<dyn Error>> {
        match change {
            InTrash::Renumbered => assert!(super::super::super::fs_quirks::evict(trashed)),
            InTrash::NumberReused => {
                fs::remove_file(trashed)?;
                self.make(trashed, b"original")?;
                fs::File::options()
                    .write(true)
                    .open(trashed)?
                    .set_modified(modified)?;
                super::super::super::fs_quirks::recreated(trashed);
            }
            InTrash::TrashedAfter => self.put_in_trash("report.2", b"later", "2026-01-02")?,
            InTrash::TrashedInTheSameTick(contents) => {
                self.put_in_trash("report.2", contents, "2026-01-01")?;
            }
        }
        Ok(())
    }
}

/// A mount, whether `report` is a folder, what changes in Trash after it
/// goes there, and which entry undo then takes for it.
type PassedOver = (Preset, bool, InTrash, Chosen);

fn check_passed_over((preset, folder, change, chosen): PassedOver) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let _quirks = FsQuirks::install(root.path(), preset());
    let room = TrashRoom {
        trash_root: trash_in(root.path()),
        original: root.path().join("report"),
        folder,
    };
    let original = &room.original;
    room.make(original, b"original")?;
    let recorded = super::super::super::super::native_fs::identity_at_path(original)
        .ok_or("recorded identity")?;
    fs::create_dir_all(room.trash_root.join("files"))?;
    fs::create_dir_all(room.trash_root.join("info"))?;
    if let InTrash::TrashedInTheSameTick(contents) = change {
        room.put_in_trash("report.1", contents, "2026-01-01")?;
    }
    room.info("report", "2026-01-01")?;
    let trashed = room.trash_root.join("files/report");
    let modified = fs::symlink_metadata(original)?.modified()?;
    fs::rename(original, &trashed)?;
    super::super::super::super::native_fs::identity_at_path(&trashed).ok_or("seen in Trash")?;
    room.change(change, &trashed, modified)?;
    let entries = trash_entries_at(
        std::slice::from_ref(&room.trash_root),
        &HashSet::from([original.clone()]),
        &gio::Cancellable::new(),
        &HashMap::from([(Location::local(original), recorded)]),
    );
    let expected = match chosen {
        Chosen::Item(name) => Some(Ok(Location::local(
            room.trash_root.join("files").join(name),
        ))),
        Chosen::Nothing => None,
        Chosen::Refused => Some(Err(
            "More than one item in Trash could be “report”, so they were left there",
        )),
    };
    assert_eq!(
        entries.get(original).map(|found| found
            .as_ref()
            .map(|entry| entry.source.clone())
            .map_err(glib::Error::message)),
        expected,
        "{:?}, folder {folder}, {change:?}",
        preset().mount_type
    );
    Ok(())
}

#[test]
fn a_trashed_original_is_passed_over_only_where_its_mount_proves_another()
-> Result<(), Box<dyn Error>> {
    use Chosen::{Item, Nothing, Refused};
    let _serial = serial()?;
    if !base_offers(&FILE_HANDLES)? {
        return Ok(());
    }
    let reusing_ext4: Preset = || Quirks {
        reused_inodes: Some("report*"),
        ..Quirks::ext4()
    };
    let reusing_nfs3: Preset = || Quirks {
        reused_inodes: Some("report*"),
        ..Quirks::nfs3()
    };
    let exfat_in_one_tick: Preset = || Quirks {
        coarse_clock: Some(SystemTime::now() - Duration::from_secs(1)),
        ..Quirks::exfat()
    };
    let same_tick = InTrash::TrashedInTheSameTick;
    let rows: [PassedOver; 8] = [
        (Quirks::vfat, false, InTrash::Renumbered, Item("report")),
        (Quirks::sshfs, false, InTrash::Renumbered, Item("report")),
        (Quirks::sshfs, true, InTrash::Renumbered, Item("report")),
        (Quirks::sshfs, true, InTrash::TrashedAfter, Item("report")),
        (reusing_ext4, false, InTrash::NumberReused, Nothing),
        (reusing_nfs3, false, InTrash::NumberReused, Nothing),
        (
            exfat_in_one_tick,
            false,
            same_tick(b"other"),
            Item("report"),
        ),
        (exfat_in_one_tick, false, same_tick(b"original"), Refused),
    ];
    for row in rows {
        check_passed_over(row)?;
    }
    Ok(())
}
