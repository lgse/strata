// SPDX-License-Identifier: MIT

use std::os::unix::fs::PermissionsExt;

use super::*;

#[derive(Clone, Copy, Debug)]
enum VolumeTrash {
    PerUser,
    Shared,
}

/// A mounted volume with a user Trash, the `folder/report.txt` original
/// and its `other.txt` sibling, and a `planted` Trash elsewhere on it.
struct Volume {
    trash: PathBuf,
    original: PathBuf,
    other: PathBuf,
    info: PathBuf,
    planted: PathBuf,
    shared: PathBuf,
}

impl Volume {
    fn create(root: &Path, layout: VolumeTrash, uid: u32) -> io::Result<Self> {
        let shared = root.join(".Trash");
        let trash = match layout {
            VolumeTrash::PerUser => root.join(format!(".Trash-{uid}")),
            VolumeTrash::Shared => {
                sticky_folder(&shared)?;
                shared.join(uid.to_string())
            }
        };
        for folder in [trash.join("files"), trash.join("info"), root.join("folder")] {
            fs::create_dir_all(folder)?;
        }
        Ok(Self {
            info: trash.join("info/report.txt.trashinfo"),
            trash,
            original: root.join("folder/report.txt"),
            other: root.join("folder/other.txt"),
            planted: root.join("planted"),
            shared,
        })
    }

    /// Puts `name` in `trash` as an item trashed from `folder/from`.
    fn trash_as(
        trash: &Path,
        (name, from): (&str, &str),
        contents: &str,
        date: &str,
    ) -> io::Result<()> {
        fs::write(trash.join("files").join(name), contents)?;
        fs::write(
            trash.join("info").join(format!("{name}.trashinfo")),
            format!("[Trash Info]\nPath=folder/{from}\nDeletionDate={date}T00:00:00\n"),
        )
    }

    fn put_in_trash(trash: &Path, contents: &str, date: &str) -> io::Result<()> {
        Self::trash_as(trash, ("report.txt", "report.txt"), contents, date)
    }

    /// Fills Trash as `trashed` describes, returning the recorded identities
    /// of originals that were trashed or purged.
    fn plant(
        &self,
        trashed: Trashed,
        uid: u32,
    ) -> Result<HashMap<Location, ItemIdentity>, Box<dyn Error>> {
        let mut originals = HashMap::new();
        match trashed {
            Trashed::Original => Self::put_in_trash(&self.trash, "original", "2026-01-01")?,
            Trashed::Nothing => {}
            Trashed::OriginalBesideANewerOneBehindALink => self.plant_behind_a_link(uid)?,
            Trashed::OnlyAnotherFromTheSamePath => {
                fs::write(&self.original, b"original")?;
                let recorded = identity_off_main(&self.original).ok_or("recorded identity")?;
                Self::put_in_trash(&self.trash, "another", "2099-01-01")?;
                fs::remove_file(&self.original)?;
                originals.insert(Location::local(&self.original), recorded);
            }
            Trashed::TwoAlikeBeforeAnother => self.plant_two_alike(&mut originals)?,
        }
        Ok(originals)
    }

    /// The original in this Trash, and a newer one in a Trash elsewhere
    /// that the shared Trash links to.
    fn plant_behind_a_link(&self, uid: u32) -> io::Result<()> {
        Self::put_in_trash(&self.trash, "original", "2026-01-01")?;
        fs::create_dir_all(self.planted.join("files"))?;
        fs::create_dir_all(self.planted.join("info"))?;
        Self::put_in_trash(&self.planted, "planted", "2099-01-01")?;
        sticky_folder(&self.shared)?;
        std::os::unix::fs::symlink(&self.planted, self.shared.join(uid.to_string()))
    }

    fn plant_two_alike(
        &self,
        originals: &mut HashMap<Location, ItemIdentity>,
    ) -> Result<(), Box<dyn Error>> {
        for (path, contents) in [(&self.original, "original"), (&self.other, "other")] {
            fs::write(path, contents)?;
            let recorded = identity_off_main(path).ok_or("recorded identity")?;
            originals.insert(Location::local(path), recorded);
            fs::remove_file(path)?;
        }
        Self::put_in_trash(&self.trash, "original", "2026-01-01")?;
        let trash = &self.trash;
        Self::trash_as(
            trash,
            ("report.2.txt", "report.txt"),
            "original",
            "2026-01-01",
        )?;
        Self::trash_as(trash, ("other.txt", "other.txt"), "other", "2026-01-01")?;
        Ok(())
    }
}

fn sticky_folder(path: &Path) -> io::Result<()> {
    fs::create_dir(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o1777))
}

fn restore_ended(event: &OperationEvent) -> bool {
    matches!(
        event,
        OperationEvent::Restored { .. }
            | OperationEvent::RestoreCompletedWithErrors { .. }
            | OperationEvent::CompletedWithErrors { .. }
            | OperationEvent::Failed { .. }
            | OperationEvent::Cancelled { .. }
    )
}

/// Undoes `undone` of the volume's originals and waits until it ends,
/// returning the operation with what it emitted.
fn undo(
    undone: Undone,
    volume: &Volume,
    trashed: Trashed,
    originals: HashMap<Location, ItemIdentity>,
) -> Result<(LoadHandle, Vec<OperationEvent>), Box<dyn Error>> {
    let location = Location::local(&volume.original);
    let locations = match trashed {
        Trashed::TwoAlikeBeforeAnother => vec![location.clone(), Location::local(&volume.other)],
        _ => vec![location.clone()],
    };
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let emit: Rc<dyn Fn(OperationEvent)> = Rc::new(move |event| emitted.borrow_mut().push(event));
    let id = OperationRequestId(1729);
    let operation = match undone {
        Undone::Replace => {
            fs::write(&volume.original, b"incoming copy")?;
            LocalOperationProvider.undo_merge(
                crate::services::UndoMergeRequest {
                    id,
                    created: Vec::new(),
                    overwritten: vec![location],
                    originals,
                    placed: placed_now(&[&volume.original]),
                },
                emit,
            )
        }
        Undone::Delete | Undone::Merge => LocalOperationProvider.restore(
            RestoreRequest {
                id,
                source: RestoreSource::OriginalLocations {
                    locations,
                    originals,
                },
            },
            emit,
        ),
    };
    wait_for_operation(&events, restore_ended);
    Ok((operation, events.take()))
}

fn assert_nothing_found(events: &[OperationEvent], volume: &Volume, trashed: Trashed, row: &str) {
    assert!(
        matches!(
            events.last(),
            Some(OperationEvent::Failed { message, .. })
                if message == "Unable to find items in Trash: \
                    One or more recently trashed items are no longer available"
        ),
        "{row}: {:?}",
        events.last()
    );
    if let Trashed::OnlyAnotherFromTheSamePath = trashed {
        assert_eq!(
            fs::read(volume.trash.join("files/report.txt")).expect("left in Trash"),
            b"another",
            "{row}"
        );
    }
}

fn restore_progress(events: &[OperationEvent]) -> Vec<(usize, Option<Location>)> {
    events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::RestoreProgress {
                completed,
                restored_location,
                ..
            } => Some((*completed, restored_location.clone())),
            _ => None,
        })
        .collect()
}

fn assert_two_alike_left(events: &[OperationEvent], volume: &Volume, row: &str) -> io::Result<()> {
    let other = Location::local(&volume.other);
    assert!(
        matches!(
            events.last(),
            Some(OperationEvent::RestoreCompletedWithErrors { restored, message, .. })
                if restored == std::slice::from_ref(&other)
                    && message.ends_with(
                        "• report.txt: More than one item in Trash could be \
                            “report.txt”, so they were left there"
                    )
        ),
        "{row}: {:?}",
        events.last()
    );
    assert_eq!(
        restore_progress(events),
        [(1, None), (2, Some(other))],
        "{row}"
    );
    assert_eq!(fs::read(&volume.other)?, b"other", "{row}");
    assert!(fs::symlink_metadata(&volume.original).is_err(), "{row}");
    for name in ["report.txt", "report.2.txt"] {
        let left = fs::read(volume.trash.join("files").join(name))?;
        assert_eq!(left, b"original", "{row}");
    }
    Ok(())
}

fn assert_restored(
    events: &[OperationEvent],
    volume: &Volume,
    trashed: Trashed,
    row: &str,
) -> io::Result<()> {
    assert!(
        matches!(events.last(), Some(OperationEvent::Restored { .. })),
        "{row}: {:?}",
        events.last()
    );
    assert_eq!(fs::read(&volume.original)?, b"original", "{row}");
    assert!(!volume.info.exists(), "{row}: the Trash entry is gone");
    if let Trashed::OriginalBesideANewerOneBehindALink = trashed {
        let planted = fs::read(volume.planted.join("files/report.txt"))?;
        assert_eq!(planted, b"planted", "{row}");
    }
    Ok(())
}

fn publish_volume(volume: &Path) {
    crate::adapters::mount_watch::publish_for_isolated_test(Some(
        crate::adapters::MountTable::parse(format!(
            "1 0 8:1 / / rw - ext4 /dev/root rw\n\
             4242 1 8:2 / {} rw - ext4 /dev/sdb1 rw\n",
            volume.display()
        )),
    ));
}

fn check_own_mount(
    layout: VolumeTrash,
    undone: Undone,
    trashed: Trashed,
) -> Result<(), Box<dyn Error>> {
    let uid = rustix::process::getuid().as_raw();
    let row = format!("{layout:?} Trash, undo of a {undone:?}, {trashed:?}");
    let root = tempfile::tempdir()?;
    let _quirks = matches!(trashed, Trashed::TwoAlikeBeforeAnother).then(|| {
        FsQuirks::install(
            root.path(),
            Quirks {
                coarse_clock: Some(SystemTime::now() - Duration::from_secs(1)),
                ..Quirks::exfat()
            },
        )
    });
    publish_volume(root.path());
    let volume = Volume::create(root.path(), layout, uid)?;
    let originals = volume.plant(trashed, uid)?;

    let (_operation, events) = undo(undone, &volume, trashed, originals)?;

    match trashed {
        Trashed::Nothing | Trashed::OnlyAnotherFromTheSamePath => {
            assert_nothing_found(&events, &volume, trashed, &row);
        }
        Trashed::TwoAlikeBeforeAnother => assert_two_alike_left(&events, &volume, &row)?,
        _ => assert_restored(&events, &volume, trashed, &row)?,
    }
    Ok(())
}

#[test]
fn undo_finds_an_original_in_the_trash_of_its_own_mount() {
    crate::test_support::isolated_test(
        "adapters::local_operations::tests::merge::undo_restore::own_mount::undo_finds_an_original_in_the_trash_of_its_own_mount",
        || {
            let _serial = serial().expect("serial");
            let per_user_delete = |trashed| (VolumeTrash::PerUser, Undone::Delete, trashed);
            for (layout, undone, trashed) in [
                (VolumeTrash::PerUser, Undone::Replace, Trashed::Original),
                (VolumeTrash::Shared, Undone::Replace, Trashed::Original),
                per_user_delete(Trashed::Original),
                per_user_delete(Trashed::Nothing),
                per_user_delete(Trashed::OriginalBesideANewerOneBehindALink),
                per_user_delete(Trashed::OnlyAnotherFromTheSamePath),
                per_user_delete(Trashed::TwoAlikeBeforeAnother),
            ] {
                check_own_mount(layout, undone, trashed).expect("undo from the volume's Trash");
            }
        },
    );
}
