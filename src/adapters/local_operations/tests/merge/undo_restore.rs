// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn merge_undo_uses_recorded_identity_when_trash_dates_collide() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for chosen in ["older.zip", "current.zip", "purged.zip"] {
        let root = tempfile::tempdir()?;
        let target = root.path().join("archive.zip");
        fs::write(&target, b"incoming archive")?;
        let mut staged = HashMap::new();
        for name in ["older.zip", "current.zip", "purged.zip"] {
            let entry = staged_trash_fixture(root.path(), name, &target, name.as_bytes())?;
            staged.insert(
                name,
                entry.source.native_path().expect("staged path").to_owned(),
            );
        }
        let original = identity_off_main(&staged[chosen]).ok_or("identity")?;
        if chosen == "purged.zip" {
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
        let lookup: StagedOriginalLookup = Rc::new(move |location, cancellable| {
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
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let placed = placed_now(&[&target]);
        glib::MainContext::default().block_on(run_merge_undo(
            OperationRequestId(91),
            (Vec::new(), vec![location], placed),
            Rc::new(move |event| emitted.borrow_mut().push(event)),
            gio::Cancellable::new(),
            lookup,
        ));

        if chosen == "purged.zip" {
            assert_eq!(fs::read(&target)?, b"incoming archive");
            assert!(matches!(
                events.borrow().last(),
                Some(OperationEvent::CompletedWithErrors { .. })
            ));
        } else {
            assert_eq!(fs::read(&target)?, chosen.as_bytes());
            assert!(matches!(
                events.borrow().last(),
                Some(OperationEvent::Restored { .. })
            ));
        }
        for (name, path) in &staged {
            if *name != chosen {
                assert_eq!(fs::read(path)?, name.as_bytes());
            }
        }
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

#[test]
fn a_trashed_original_is_passed_over_only_where_its_mount_proves_another()
-> Result<(), Box<dyn Error>> {
    use Chosen::{Item, Nothing, Refused};
    let _serial = serial()?;
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
    let rows: [(Preset, bool, InTrash, Chosen); 8] = [
        (Quirks::vfat, false, InTrash::Renumbered, Item("report")),
        (Quirks::sshfs, false, InTrash::Renumbered, Item("report")),
        (Quirks::sshfs, true, InTrash::Renumbered, Item("report")),
        (Quirks::sshfs, true, InTrash::TrashedAfter, Item("report")),
        (reusing_ext4, false, InTrash::NumberReused, Nothing),
        (reusing_nfs3, false, InTrash::NumberReused, Nothing),
        (
            exfat_in_one_tick,
            false,
            InTrash::TrashedInTheSameTick(b"other"),
            Item("report"),
        ),
        (
            exfat_in_one_tick,
            false,
            InTrash::TrashedInTheSameTick(b"original"),
            Refused,
        ),
    ];
    for (preset, folder, change, chosen) in rows {
        let root = tempfile::tempdir()?;
        let _quirks = FsQuirks::install(root.path(), preset());
        let original = root.path().join("report");
        let make = |path: &Path, contents: &[u8]| {
            if folder {
                fs::create_dir(path)
            } else {
                fs::write(path, contents)
            }
        };
        make(&original, b"original")?;
        let recorded = super::super::super::native_fs::identity_at_path(&original)
            .ok_or("recorded identity")?;
        let trash_root = root
            .path()
            .join(format!(".Trash-{}", rustix::process::getuid().as_raw()));
        fs::create_dir_all(trash_root.join("files"))?;
        fs::create_dir_all(trash_root.join("info"))?;
        let put_in_trash = |name: &str, contents: &[u8], date: &str| -> io::Result<()> {
            make(&trash_root.join("files").join(name), contents)?;
            fs::write(
                trash_root.join("info").join(format!("{name}.trashinfo")),
                format!(
                    "[Trash Info]\nPath={}\nDeletionDate={date}T00:00:00\n",
                    original.display()
                ),
            )
        };
        if let InTrash::TrashedInTheSameTick(contents) = change {
            put_in_trash("report.1", contents, "2026-01-01")?;
        }
        fs::write(
            trash_root.join("info/report.trashinfo"),
            format!(
                "[Trash Info]\nPath={}\nDeletionDate=2026-01-01T00:00:00\n",
                original.display()
            ),
        )?;
        let trashed = trash_root.join("files/report");
        let modified = fs::symlink_metadata(&original)?.modified()?;
        fs::rename(&original, &trashed)?;
        super::super::super::native_fs::identity_at_path(&trashed).ok_or("seen in Trash")?;
        match change {
            InTrash::Renumbered => assert!(super::super::fs_quirks::evict(&trashed)),
            InTrash::NumberReused => {
                fs::remove_file(&trashed)?;
                make(&trashed, b"original")?;
                fs::File::options()
                    .write(true)
                    .open(&trashed)?
                    .set_modified(modified)?;
                super::super::fs_quirks::recreated(&trashed);
            }
            InTrash::TrashedAfter => put_in_trash("report.2", b"later", "2026-01-02")?,
            InTrash::TrashedInTheSameTick(contents) => {
                put_in_trash("report.2", contents, "2026-01-01")?;
            }
        }
        let entries = trash_entries_at(
            std::slice::from_ref(&trash_root),
            &HashSet::from([original.clone()]),
            &gio::Cancellable::new(),
            &HashMap::from([(Location::local(&original), recorded)]),
        );
        let expected = match chosen {
            Item(name) => Some(Ok(Location::local(trash_root.join("files").join(name)))),
            Nothing => None,
            Refused => Some(Err(
                "More than one item in Trash could be “report”, so they were left there",
            )),
        };
        assert_eq!(
            entries.get(&original).map(|found| found
                .as_ref()
                .map(|entry| entry.source.clone())
                .map_err(glib::Error::message)),
            expected,
            "{:?}, folder {folder}, {change:?}",
            preset().mount_type
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum VolumeTrash {
    PerUser,
    Shared,
}

#[test]
fn undo_finds_an_original_in_the_trash_of_its_own_mount() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::merge::undo_finds_an_original_in_the_trash_of_its_own_mount",
        || {
            let _serial = serial().expect("serial");
            let uid = rustix::process::getuid().as_raw();
            for (layout, undone, trashed) in [
                (VolumeTrash::PerUser, Undone::Replace, Trashed::Original),
                (VolumeTrash::Shared, Undone::Replace, Trashed::Original),
                (VolumeTrash::PerUser, Undone::Delete, Trashed::Original),
                (VolumeTrash::PerUser, Undone::Delete, Trashed::Nothing),
                (
                    VolumeTrash::PerUser,
                    Undone::Delete,
                    Trashed::OriginalBesideANewerOneBehindALink,
                ),
                (
                    VolumeTrash::PerUser,
                    Undone::Delete,
                    Trashed::OnlyAnotherFromTheSamePath,
                ),
                (
                    VolumeTrash::PerUser,
                    Undone::Delete,
                    Trashed::TwoAlikeBeforeAnother,
                ),
            ] {
                let row = format!("{layout:?} Trash, undo of a {undone:?}, {trashed:?}");
                let volume = tempfile::tempdir().expect("volume");
                let _quirks = matches!(trashed, Trashed::TwoAlikeBeforeAnother).then(|| {
                    FsQuirks::install(
                        volume.path(),
                        Quirks {
                            coarse_clock: Some(SystemTime::now() - Duration::from_secs(1)),
                            ..Quirks::exfat()
                        },
                    )
                });
                crate::adapters::mount_watch::publish_for_isolated_test(Some(
                    crate::adapters::MountTable::parse(format!(
                        "1 0 8:1 / / rw - ext4 /dev/root rw\n\
                         4242 1 8:2 / {} rw - ext4 /dev/sdb1 rw\n",
                        volume.path().display()
                    )),
                ));
                let trash = match layout {
                    VolumeTrash::PerUser => volume.path().join(format!(".Trash-{uid}")),
                    VolumeTrash::Shared => {
                        let shared = volume.path().join(".Trash");
                        fs::create_dir(&shared).expect("shared Trash");
                        fs::set_permissions(
                            &shared,
                            std::os::unix::fs::PermissionsExt::from_mode(0o1777),
                        )
                        .expect("sticky shared Trash");
                        shared.join(uid.to_string())
                    }
                };
                for folder in [
                    trash.join("files"),
                    trash.join("info"),
                    volume.path().join("folder"),
                ] {
                    fs::create_dir_all(folder).expect("fixture folder");
                }
                let original = volume.path().join("folder/report.txt");
                let other = volume.path().join("folder/other.txt");
                let info = trash.join("info/report.txt.trashinfo");
                let trash_as =
                    |trash: &Path, name: &str, from: &str, contents: &str, date: &str| {
                        fs::write(trash.join("files").join(name), contents).expect("trashed item");
                        fs::write(
                            trash.join("info").join(format!("{name}.trashinfo")),
                            format!(
                                "[Trash Info]\nPath=folder/{from}\nDeletionDate={date}T00:00:00\n"
                            ),
                        )
                        .expect("trash info");
                    };
                let put_in_trash = |trash: &Path, contents: &str, date: &str| {
                    trash_as(trash, "report.txt", "report.txt", contents, date);
                };
                let planted = volume.path().join("planted");
                let mut originals = HashMap::new();
                match trashed {
                    Trashed::Original => put_in_trash(&trash, "original", "2026-01-01"),
                    Trashed::Nothing => {}
                    Trashed::OriginalBesideANewerOneBehindALink => {
                        put_in_trash(&trash, "original", "2026-01-01");
                        fs::create_dir_all(planted.join("files")).expect("planted files");
                        fs::create_dir_all(planted.join("info")).expect("planted info");
                        put_in_trash(&planted, "planted", "2099-01-01");
                        let shared = volume.path().join(".Trash");
                        fs::create_dir(&shared).expect("shared Trash");
                        fs::set_permissions(
                            &shared,
                            std::os::unix::fs::PermissionsExt::from_mode(0o1777),
                        )
                        .expect("sticky shared Trash");
                        std::os::unix::fs::symlink(&planted, shared.join(uid.to_string()))
                            .expect("linked user Trash");
                    }
                    Trashed::OnlyAnotherFromTheSamePath => {
                        fs::write(&original, b"original").expect("original");
                        let recorded = identity_off_main(&original).expect("recorded identity");
                        put_in_trash(&trash, "another", "2099-01-01");
                        fs::remove_file(&original).expect("original purged");
                        originals.insert(Location::local(&original), recorded);
                    }
                    Trashed::TwoAlikeBeforeAnother => {
                        for (path, contents) in [(&original, "original"), (&other, "other")] {
                            fs::write(path, contents).expect("trashed original");
                            let recorded = identity_off_main(path).expect("recorded identity");
                            originals.insert(Location::local(path), recorded);
                            fs::remove_file(path).expect("original trashed");
                        }
                        put_in_trash(&trash, "original", "2026-01-01");
                        trash_as(
                            &trash,
                            "report.2.txt",
                            "report.txt",
                            "original",
                            "2026-01-01",
                        );
                        trash_as(&trash, "other.txt", "other.txt", "other", "2026-01-01");
                    }
                }
                let location = Location::local(&original);
                let locations = match trashed {
                    Trashed::TwoAlikeBeforeAnother => {
                        vec![location.clone(), Location::local(&other)]
                    }
                    _ => vec![location.clone()],
                };
                let events = Rc::new(RefCell::new(Vec::new()));
                let emitted = events.clone();
                let emit: Rc<dyn Fn(OperationEvent)> =
                    Rc::new(move |event| emitted.borrow_mut().push(event));
                let id = OperationRequestId(1729);
                let _operation = match undone {
                    Undone::Replace => {
                        fs::write(&original, b"incoming copy").expect("incoming copy");
                        LocalOperationProvider.undo_merge(
                            crate::services::UndoMergeRequest {
                                id,
                                created: Vec::new(),
                                overwritten: vec![location.clone()],
                                originals,
                                placed: placed_now(&[&original]),
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
                wait_for_operation(&events, |event| {
                    matches!(
                        event,
                        OperationEvent::Restored { .. }
                            | OperationEvent::RestoreCompletedWithErrors { .. }
                            | OperationEvent::CompletedWithErrors { .. }
                            | OperationEvent::Failed { .. }
                            | OperationEvent::Cancelled { .. }
                    )
                });

                if let Trashed::Nothing | Trashed::OnlyAnotherFromTheSamePath = trashed {
                    assert!(
                        matches!(
                            events.borrow().last(),
                            Some(OperationEvent::Failed { message, .. })
                                if message == "Unable to find items in Trash: \
                                    One or more recently trashed items are no longer available"
                        ),
                        "{row}: {:?}",
                        events.borrow().last()
                    );
                    if let Trashed::OnlyAnotherFromTheSamePath = trashed {
                        assert_eq!(
                            fs::read(trash.join("files/report.txt")).expect("left in Trash"),
                            b"another",
                            "{row}"
                        );
                    }
                    continue;
                }
                if let Trashed::TwoAlikeBeforeAnother = trashed {
                    assert!(
                        matches!(
                            events.borrow().last(),
                            Some(OperationEvent::RestoreCompletedWithErrors { restored, message, .. })
                                if restored == &[Location::local(&other)]
                                    && message.ends_with(
                                        "• report.txt: More than one item in Trash could be \
                                            “report.txt”, so they were left there"
                                    )
                        ),
                        "{row}: {:?}",
                        events.borrow().last()
                    );
                    let progress = events
                        .borrow()
                        .iter()
                        .filter_map(|event| match event {
                            OperationEvent::RestoreProgress {
                                completed,
                                restored_location,
                                ..
                            } => Some((*completed, restored_location.clone())),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(
                        progress,
                        [(1, None), (2, Some(Location::local(&other)))],
                        "{row}"
                    );
                    assert_eq!(fs::read(&other).expect("restored"), b"other", "{row}");
                    assert!(fs::symlink_metadata(&original).is_err(), "{row}");
                    for name in ["report.txt", "report.2.txt"] {
                        assert_eq!(
                            fs::read(trash.join("files").join(name)).expect("left in Trash"),
                            b"original",
                            "{row}"
                        );
                    }
                    continue;
                }
                assert!(
                    matches!(
                        events.borrow().last(),
                        Some(OperationEvent::Restored { .. })
                    ),
                    "{row}: {:?}",
                    events.borrow().last()
                );
                assert_eq!(fs::read(&original).expect("restored"), b"original", "{row}");
                assert!(!info.exists(), "{row}: the Trash entry is gone");
                if let Trashed::OriginalBesideANewerOneBehindALink = trashed {
                    assert_eq!(
                        fs::read(planted.join("files/report.txt")).expect("planted"),
                        b"planted",
                        "{row}"
                    );
                }
            }
        },
    );
}

#[test]
fn merge_undo_restores_staged_originals_and_removes_created() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::merge::merge_undo_restores_staged_originals_and_removes_created",
        || check_merge_undo_restores_staged_originals_and_removes_created().expect("merge undo"),
    );
}

fn check_merge_undo_restores_staged_originals_and_removes_created() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let target = root.path().join("target");
    fs::create_dir_all(&target)?;
    let shared = target.join("shared.txt");
    let incoming = target.join("incoming.txt");
    let stays = target.join("stays.txt");
    fs::write(&shared, b"incoming wins")?;
    fs::write(&incoming, b"new")?;
    fs::write(&stays, b"keep me")?;
    let bookmarks = crate::adapters::bookmarks::pinned_places_path();
    fs::create_dir_all(bookmarks.parent().expect("bookmarks directory"))?;
    let pin = |path: &Path| format!("{} Pin\n", gio::File::for_path(path).uri());
    fs::write(
        &bookmarks,
        format!("{}{}{}", pin(&shared), pin(&incoming), pin(&stays)),
    )?;
    let incoming_identity = numbers_at(&incoming);
    let staged = staged_trash_fixture(root.path(), "shared.txt", &shared, b"old")?;
    let staged_path = staged
        .source
        .native_path()
        .expect("staged path")
        .to_path_buf();
    let lookup = staged_lookup(HashMap::from([(shared.clone(), staged)]));

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let placed = placed_now(&[&shared]);
    glib::MainContext::default().block_on(run_merge_undo(
        OperationRequestId(90),
        (
            vec![Location::local(&incoming)],
            vec![Location::local(&shared)],
            placed,
        ),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
        gio::Cancellable::new(),
        lookup,
    ));

    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::Restored { .. })
        ),
        "{:?}",
        events.borrow()
    );
    assert_eq!(
        fs::read(&shared)?,
        b"old",
        "the staged original is restored"
    );
    assert!(!incoming.exists(), "the merge-created file is removed");
    assert_eq!(
        trashed_items(&events.borrow()),
        [(Location::local(&incoming), incoming_identity)]
    );
    assert!(!staged_path.exists(), "the staged copy left the fake trash");
    assert_eq!(fs::read(&stays)?, b"keep me");
    assert!(target.exists(), "the destination folder itself survives");
    crate::storage::writer::settle();
    assert_eq!(
        fs::read_to_string(bookmarks)?,
        format!("{}{}", pin(&shared), pin(&stays))
    );
    Ok(())
}

#[test]
fn undoing_a_merge_with_numbered_aliases_removes_only_its_items() -> Result<(), Box<dyn Error>> {
    use crate::services::{RenameCause, RenamedItem};
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    let folder = destination.join("folder");
    fs::create_dir_all(sources.join("folder"))?;
    fs::create_dir_all(&folder)?;
    for name in ["Makefile", "makefile"] {
        fs::write(sources.join("folder").join(name), name)?;
    }
    fs::write(folder.join("keep.txt"), b"keep me")?;
    // Rules that do not fold case learn that a name is taken only when it is refused.
    let _scope = FsQuirks::install(
        &destination,
        without_trash(Quirks {
            mount_type: Some(("fuse.ciopfs", "rw")),
            case_insensitive: true,
            ..Quirks::default()
        }),
    );

    let events = run_paste(PasteRequest {
        id: OperationRequestId(94),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(sources.join("folder")),
            conflict: TransferConflict::Merge,
            target_name: None,
        }],
        move_sources: false,
    });

    let mut created = events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::Merged { created, .. } => Some(created.clone()),
            _ => None,
        })
        .flatten()
        .filter_map(|location| location.native_path().map(Path::to_path_buf))
        .collect::<Vec<_>>();
    created.sort();
    assert_eq!(created, [folder.join("Makefile"), folder.join("makefile")]);
    let renamed = events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::ItemsReported { report, .. } => Some(report.renamed.clone()),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();
    let saved_as = Location::local(folder.join("makefile (1)"));
    assert_eq!(
        renamed,
        [RenamedItem {
            requested: Location::local(folder.join("makefile")),
            saved_as: saved_as.clone(),
            cause: RenameCause::Alias,
        }]
    );
    let placed = events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::Merged { placed, .. } => Some(placed.clone()),
            _ => None,
        })
        .flatten()
        .collect::<HashMap<_, _>>();
    assert!(
        placed.contains_key(&saved_as),
        "an alias is recorded as saved"
    );
    let undone = Rc::new(RefCell::new(Vec::new()));
    let emitted = undone.clone();
    glib::MainContext::default().block_on(run_merge_undo(
        OperationRequestId(95),
        (
            vec![Location::local(folder.join("Makefile")), saved_as],
            Vec::new(),
            placed,
        ),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
        gio::Cancellable::new(),
        staged_lookup(HashMap::new()),
    ));

    assert!(
        matches!(
            undone.borrow().last(),
            Some(OperationEvent::Restored { .. })
        ),
        "{:?}",
        undone.borrow()
    );
    assert_eq!(names(&folder)?, ["keep.txt"]);
    Ok(())
}

#[test]
fn merge_undo_keeps_the_incoming_copy_when_no_staged_original_exists() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let target = root.path().join("target");
    fs::create_dir_all(&target)?;
    let shared = target.join("shared.txt");
    fs::write(&shared, b"incoming wins")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let placed = placed_now(&[&shared]);
    glib::MainContext::default().block_on(run_merge_undo(
        OperationRequestId(91),
        (Vec::new(), vec![Location::local(&shared)], placed),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
        gio::Cancellable::new(),
        staged_lookup(HashMap::new()),
    ));

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::CompletedWithErrors { .. })
    ));
    assert_eq!(
        fs::read(&shared)?,
        b"incoming wins",
        "the lookup runs before the incoming copy is deleted"
    );
    Ok(())
}

#[test]
fn merge_undo_tolerates_created_paths_that_no_longer_exist() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let target = root.path().join("target");
    fs::create_dir_all(&target)?;
    let gone = target.join("already-removed.txt");

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    glib::MainContext::default().block_on(run_merge_undo(
        OperationRequestId(92),
        (vec![Location::local(&gone)], Vec::new(), HashMap::new()),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
        gio::Cancellable::new(),
        staged_lookup(HashMap::new()),
    ));

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Restored { locations, .. }) if locations == &[Location::local(&gone)]
    ));
    Ok(())
}

#[test]
fn a_cancelled_merge_undo_reports_everything_not_attempted() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let target = root.path().join("target");
    fs::create_dir_all(&target)?;
    let shared = target.join("shared.txt");
    let incoming = target.join("incoming.txt");
    fs::write(&shared, b"incoming wins")?;
    fs::write(&incoming, b"new")?;
    let cancellable = gio::Cancellable::new();
    cancellable.cancel();

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    glib::MainContext::default().block_on(run_merge_undo(
        OperationRequestId(93),
        (
            vec![Location::local(&incoming)],
            vec![Location::local(&shared)],
            HashMap::new(),
        ),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
        cancellable,
        staged_lookup(HashMap::new()),
    ));

    let events = events.borrow();
    let Some(OperationEvent::Cancelled { result, .. }) = events.last() else {
        panic!("expected cancellation, got {events:?}");
    };
    assert!(result.completed.is_empty());
    assert_eq!(
        result.not_attempted,
        vec![Location::local(&shared), Location::local(&incoming)]
    );
    assert_eq!(fs::read(&shared)?, b"incoming wins");
    assert_eq!(fs::read(&incoming)?, b"new");
    Ok(())
}

#[test]
fn a_merge_a_cancel_cuts_off_counts_as_failed_as_it_keeps_what_it_placed()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir_all(sources.join("run"))?;
    fs::create_dir_all(destination.join("run"))?;
    fs::write(sources.join("run/big.bin"), vec![0x5a; 4 * 1024 * 1024])?;
    fs::write(destination.join("run/old.txt"), b"old")?;
    let cancel = CancelFromMain::default();
    let _sources = FsQuirks::install(&sources, Quirks::default());
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            before: vec![
                (Op::CopyRange, "*", 0, cancel.racer()),
                (Op::Write, "*", 0, cancel.racer()),
            ],
            reflink: super::super::fs_quirks::Support::Rejected(Errno::OPNOTSUPP),
            ..Quirks::default()
        },
    );
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let mut operation = Some(LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(1615),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(sources.join("run")),
                conflict: TransferConflict::Merge,
                target_name: None,
            }],
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    ));

    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Cancelled { .. }
                | OperationEvent::Pasted { .. }
                | OperationEvent::TransferFailed { .. }
        )
    }) {
        assert!(Instant::now() < deadline, "the merge never ended");
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
    assert_eq!(result.failed, [Location::local(sources.join("run"))]);
    assert!(result.not_attempted.is_empty());
    assert_eq!(fs::read(destination.join("run/old.txt"))?, b"old");
    Ok(())
}

#[test]
fn a_merge_without_trash_cancelled_after_it_placed_a_copy_puts_the_original_back()
-> Result<(), Box<dyn Error>> {
    #[derive(Clone, Copy, Debug)]
    enum Copied {
        AsPlaced,
        Rewritten,
        RenamedOver,
    }
    let _serial = serial()?;
    let mut rows = [false, true]
        .into_iter()
        .flat_map(|moving| {
            [Copied::AsPlaced, Copied::Rewritten, Copied::RenamedOver]
                .map(|copy| (moving, copy, Quirks::default as Preset))
        })
        .collect::<Vec<_>>();
    rows.push((false, Copied::AsPlaced, Quirks::exfat));
    for (move_sources, copy, preset) in rows {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        write_files(
            &sources.join("run"),
            &[("a.txt", "new a"), ("b.txt", "new b"), ("c.txt", "new c")],
        )?;
        write_files(
            &destination.join("run"),
            &[("a.txt", "old a"), ("b.txt", "old b")],
        )?;
        let cancel = CancelFromMain::default();
        let mut cancels = cancel.racer();
        let (placed, theirs) = (destination.join("run/a.txt"), root.path().join("theirs"));
        let racer: Racer = Box::new(move || {
            match copy {
                Copied::AsPlaced => {}
                Copied::Rewritten => fs::write(&placed, b"theirs, longer").expect("rewrite"),
                Copied::RenamedOver => {
                    fs::write(&theirs, b"theirs, longer").expect("write theirs");
                    fs::rename(&theirs, &placed).expect("rename theirs over the copy");
                }
            }
            cancels();
        });
        let _sources = FsQuirks::install(&sources, Quirks::default());
        let _destination = FsQuirks::install(
            &destination,
            without_trash(Quirks {
                after: vec![(Op::Rename, "a.txt", 1, racer)],
                ..preset()
            }),
        );
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let mut operation = Some(LocalOperationProvider.paste(
            PasteRequest {
                id: OperationRequestId(1729),
                destination: Location::local(&destination),
                items: vec![PasteItem {
                    source: Location::local(sources.join("run")),
                    conflict: TransferConflict::Merge,
                    target_name: None,
                }],
                move_sources,
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        ));

        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !events.borrow().iter().any(|event| {
            matches!(
                event,
                OperationEvent::Cancelled { .. }
                    | OperationEvent::Pasted { .. }
                    | OperationEvent::TransferFailed { .. }
            )
        }) {
            assert!(Instant::now() < deadline, "the merge never ended");
            cancel.serve(|| drop(operation.take()));
            context.iteration(false);
            thread::sleep(Duration::from_millis(1));
        }

        let row = format!(
            "moving: {move_sources}, {copy:?}, {:?}",
            preset().mount_type
        );
        let events = events.borrow();
        assert!(cancel.served(), "{row}");
        assert_eq!(fs::read(destination.join("run/b.txt"))?, b"old b", "{row}");
        if let Copied::AsPlaced = copy {
            let Some(OperationEvent::Cancelled { result, .. }) = events.last() else {
                panic!("{row}: expected a cancel, got {events:?}");
            };
            assert!(result.completed.is_empty(), "{row}");
            assert_eq!(
                result.failed,
                [Location::local(sources.join("run"))],
                "{row}"
            );
            assert_eq!(fs::read(destination.join("run/a.txt"))?, b"old a", "{row}");
            assert_eq!(
                names(&destination.join("run"))?,
                ["a.txt", "b.txt"],
                "{row}"
            );
        } else {
            let Some(OperationEvent::TransferFailed { message, .. }) = events.last() else {
                panic!("{row}: expected the merge to stop, got {events:?}");
            };
            let left = names(&destination.join("run"))?;
            let [folder, a, b] = &left[..] else {
                panic!("{row}: expected the merge folder beside a.txt and b.txt, found {left:?}");
            };
            assert!(folder.starts_with(".strata-merge-"), "{row}: {left:?}");
            assert_eq!([a.as_str(), b.as_str()], ["a.txt", "b.txt"], "{row}");
            assert!(message.contains(folder.as_str()), "{row}: {message}");
            assert_eq!(
                fs::read(destination.join("run/a.txt"))?,
                b"theirs, longer",
                "{row}"
            );
            assert_eq!(
                fs::read(destination.join("run").join(folder).join("a.txt"))?,
                b"old a",
                "{row}"
            );
        }
        for (name, contents) in [("a.txt", "new a"), ("b.txt", "new b"), ("c.txt", "new c")] {
            assert_eq!(
                fs::read(sources.join("run").join(name))?,
                contents.as_bytes(),
                "{row}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_merge_undo_cut_off_counts_an_item_as_not_attempted_only_when_it_left_it_whole()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // Without Trash, the first check is Trash's, the second the comparison's
    // and the third the delete's.
    for (row, check) in [("Trash", None), ("compare", Some(1)), ("delete", Some(2))] {
        let root = tempfile::tempdir()?;
        let created = ["first.txt", "second.txt"].map(|name| root.path().join(name));
        for item in &created {
            fs::write(item, b"merged")?;
        }
        let placed = placed_now(&[&created[0], &created[1]]);
        let cancellable = gio::Cancellable::new();
        let cancel = cancellable.clone();
        let cancel: super::super::fs_quirks::Racer = Box::new(move || cancel.cancel());
        let quirks = match check {
            None => Quirks {
                before: vec![(Op::Trash, "first.txt", 0, cancel)],
                ..Quirks::default()
            },
            Some(check) => without_trash(Quirks {
                after: vec![(Op::Stat, "first.txt", check, cancel)],
                ..Quirks::default()
            }),
        };
        let _scope = FsQuirks::install(root.path(), quirks);
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();

        glib::MainContext::default().block_on(run_merge_undo(
            OperationRequestId(1615),
            (
                created.iter().map(Location::local).collect(),
                Vec::new(),
                placed,
            ),
            Rc::new(move |event| emitted.borrow_mut().push(event)),
            cancellable,
            staged_lookup(HashMap::new()),
        ));

        let events = events.borrow();
        let Some(OperationEvent::Cancelled { result, .. }) = events.last() else {
            panic!("{row}: expected a cancel, got {events:?}");
        };
        let [first, second] = &created;
        assert!(result.completed.is_empty(), "{row}");
        if row == "delete" {
            assert!(!first.exists(), "the cancel landed before the delete");
            assert_eq!(result.failed, [Location::local(first)]);
            assert_eq!(result.not_attempted, [Location::local(second)]);
        } else {
            assert!(result.failed.is_empty(), "{row}");
            assert_eq!(
                result.not_attempted,
                [Location::local(first), Location::local(second)],
                "{row}"
            );
            assert_eq!(fs::read(first)?, b"merged", "{row}");
        }
        assert_eq!(fs::read(second)?, b"merged", "{row}");
    }
    Ok(())
}

#[test]
fn a_kept_item_counts_as_failed_when_a_later_item_is_cut_off() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let [kept, cut] = ["kept.txt", "cut.txt"].map(|name| root.path().join(name));
    for item in [&kept, &cut] {
        fs::write(item, b"merged")?;
    }
    let placed = placed_now(&[&cut]);
    let cancellable = gio::Cancellable::new();
    let cancel = cancellable.clone();
    let cancel: Racer = Box::new(move || cancel.cancel());
    let _scope = FsQuirks::install(
        root.path(),
        without_trash(Quirks {
            after: vec![(Op::Stat, "cut.txt", 1, cancel)],
            ..Quirks::default()
        }),
    );
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();

    glib::MainContext::default().block_on(run_merge_undo(
        OperationRequestId(1617),
        (
            vec![Location::local(&kept), Location::local(&cut)],
            Vec::new(),
            placed,
        ),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
        cancellable,
        staged_lookup(HashMap::new()),
    ));

    let events = events.borrow();
    let Some(OperationEvent::Cancelled { result, .. }) = events.last() else {
        panic!("expected a cancel, got {events:?}");
    };
    assert!(result.completed.is_empty(), "{result:?}");
    assert_eq!(result.failed, [Location::local(&kept)]);
    assert_eq!(result.not_attempted, [Location::local(&cut)]);
    assert_eq!(kept_items(&events), [Location::local(&kept)]);
    assert_eq!(fs::read(&kept)?, b"merged");
    assert_eq!(fs::read(&cut)?, b"merged");
    Ok(())
}

#[test]
fn a_merge_undo_never_counts_an_item_it_could_not_check_as_undone() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let [first, second] = ["first.txt", "second.txt"].map(|name| root.path().join(name));
    for item in [&first, &second] {
        fs::write(item, b"merged")?;
    }
    let unreadable = Location::uri("strata-test-no-backend:///odd.txt");
    let placed = placed_now(&[&first, &second]);
    let _scope = FsQuirks::install(root.path(), without_trash(Quirks::default()));
    let cancellable = gio::Cancellable::new();
    let cancel = cancellable.clone();
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let emit = Rc::new(move |event: OperationEvent| {
        if matches!(event, OperationEvent::DeleteProgress { completed: 2, .. }) {
            let cancel = cancel.clone();
            // Runs ahead of the reply to the next item's check, which is then in flight.
            glib::idle_add_local_full(glib::Priority::HIGH, move || {
                cancel.cancel();
                glib::ControlFlow::Break
            });
        }
        emitted.borrow_mut().push(event);
    });

    glib::MainContext::default().block_on(run_merge_undo(
        OperationRequestId(1615),
        (
            vec![
                Location::local(&first),
                unreadable.clone(),
                Location::local(&second),
            ],
            Vec::new(),
            placed,
        ),
        emit,
        cancellable,
        staged_lookup(HashMap::new()),
    ));

    let events = events.borrow();
    let Some(OperationEvent::Cancelled { result, .. }) = events.last() else {
        panic!("expected a cancel, got {events:?}");
    };
    assert_eq!(result.completed, [Location::local(&first)]);
    assert_eq!(result.failed, [unreadable]);
    assert_eq!(result.not_attempted, [Location::local(&second)]);
    assert!(!first.exists());
    assert_eq!(fs::read(&second)?, b"merged");
    Ok(())
}

#[test]
fn a_merge_undo_cancelled_while_it_restores_an_original_ends_as_a_cancel()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // Whether the merge's copy is still at the last path, which the undo
    // deletes before it restores the original, and whether the cut-off copy
    // of the original cannot be removed.
    for (incoming, stuck) in [(true, false), (false, false), (false, true)] {
        let root = tempfile::tempdir()?;
        let [first, second] = ["first.bin", "second.bin"].map(|name| root.path().join(name));
        let original = vec![0x5a; 4 * 1024 * 1024];
        fs::write(&first, b"incoming")?;
        if incoming {
            fs::write(&second, b"incoming")?;
        }
        let staged = HashMap::from([
            (
                first.clone(),
                staged_trash_fixture(root.path(), "first.bin", &first, b"first original")?,
            ),
            (
                second.clone(),
                staged_trash_fixture(root.path(), "second.bin", &second, &original)?,
            ),
        ]);
        let trash = root
            .path()
            .join(format!(".Trash-{}", rustix::process::getuid().as_raw()));
        let cancellable = gio::Cancellable::new();
        let racer = |cancel: gio::Cancellable| -> super::super::fs_quirks::Racer {
            Box::new(move || cancel.cancel())
        };
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                before: vec![
                    (Op::CopyRange, "second.bin", 0, racer(cancellable.clone())),
                    (Op::Write, "second.bin", 0, racer(cancellable.clone())),
                ],
                fail: if stuck {
                    vec![(Op::Unlink, "second.bin", 0, Errno::IO)]
                } else {
                    Vec::new()
                },
                reflink: super::super::fs_quirks::Support::Rejected(Errno::OPNOTSUPP),
                ..Quirks::without_placement()
            },
        );
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();

        let placed = placed_now(&[&first, &second]);
        glib::MainContext::default().block_on(run_merge_undo(
            OperationRequestId(1615),
            (
                Vec::new(),
                vec![Location::local(&first), Location::local(&second)],
                placed,
            ),
            Rc::new(move |event| emitted.borrow_mut().push(event)),
            cancellable,
            staged_lookup(staged),
        ));

        let events = events.borrow();
        let Some(OperationEvent::Cancelled { result, .. }) = events.last() else {
            panic!(
                "{incoming}, {stuck}: expected a cancel, got {:?}",
                events.last()
            );
        };
        let cut_off = vec![Location::local(&second)];
        let (failed, not_attempted) = if incoming || stuck {
            (cut_off, Vec::new())
        } else {
            (Vec::new(), cut_off)
        };
        assert_eq!(result.completed, [Location::local(&first)]);
        assert_eq!(result.failed, failed, "{incoming}, {stuck}");
        assert_eq!(result.not_attempted, not_attempted, "{incoming}, {stuck}");
        assert_eq!(fs::read(&first)?, b"first original");
        let left = fs::symlink_metadata(&second).is_ok();
        assert_eq!(left, stuck, "{incoming}, {stuck}");
        assert_eq!(fs::read(trash.join("files/second.bin"))?, original);
        assert!(trash.join("info/second.bin.trashinfo").exists());
    }
    Ok(())
}

#[test]
fn a_merge_undo_restores_an_original_only_over_a_path_it_checked() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unreadable = "strata-test-no-backend:///second.txt";
    // The second path's check fails, a cancel cuts it off, or the merge's
    // copy is already gone from it.
    for case in ["failed", "cancelled", "missing"] {
        let root = tempfile::tempdir()?;
        let [first, second] = ["first.txt", "second.txt"].map(|name| root.path().join(name));
        fs::write(&first, b"merged")?;
        if case == "cancelled" {
            fs::write(&second, b"merged")?;
        }
        let target = match case {
            "failed" => Location::uri(unreadable),
            _ => Location::local(&second),
        };
        let entries = RefCell::new(HashMap::from([
            (
                Location::local(&first),
                staged_trash_fixture(root.path(), "first.txt", &first, b"first")?,
            ),
            (
                target.clone(),
                staged_trash_fixture(root.path(), "second.txt", &second, b"second")?,
            ),
        ]));
        let lookup: StagedOriginalLookup = Rc::new(move |location, _| {
            let entry = entries.borrow_mut().remove(&location);
            Box::pin(async move { entry.ok_or_else(|| io_error("not staged")) })
        });
        let trash = root
            .path()
            .join(format!(".Trash-{}", rustix::process::getuid().as_raw()));
        let cancellable = gio::Cancellable::new();
        let cancel = cancellable.clone();
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let emit = Rc::new(move |event: OperationEvent| {
            if case == "cancelled"
                && matches!(event, OperationEvent::RestoreProgress { completed: 1, .. })
            {
                let cancel = cancel.clone();
                // Runs ahead of the reply to the second path's check, then in flight.
                glib::idle_add_local_full(glib::Priority::HIGH, move || {
                    cancel.cancel();
                    glib::ControlFlow::Break
                });
            }
            emitted.borrow_mut().push(event);
        });

        let placed = placed_now(&[&first]);
        glib::MainContext::default().block_on(run_merge_undo(
            OperationRequestId(1615),
            (
                Vec::new(),
                vec![Location::local(&first), target.clone()],
                placed,
            ),
            emit,
            cancellable,
            lookup,
        ));

        let events = events.borrow();
        assert_eq!(fs::read(&first)?, b"first", "{case}");
        let staged_second = fs::read(trash.join("files/second.txt"));
        match (case, events.last()) {
            ("failed", Some(OperationEvent::CompletedWithErrors { message, .. })) => {
                let check = gio::File::for_uri(unreadable)
                    .query_info(
                        "standard::type",
                        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                        gio::Cancellable::NONE,
                    )
                    .expect_err("no backend");
                assert!(
                    message.contains(&crate::services::gio_error_detail(&check)),
                    "{message}"
                );
                assert_eq!(staged_second?, b"second");
                assert!(trash.join("info/second.txt.trashinfo").exists());
                assert!(fs::symlink_metadata(&second).is_err());
            }
            ("cancelled", Some(OperationEvent::Cancelled { result, .. })) => {
                assert_eq!(result.completed, [Location::local(&first)]);
                assert!(result.failed.is_empty());
                assert_eq!(result.not_attempted, [Location::local(&second)]);
                assert_eq!(staged_second?, b"second");
                assert_eq!(fs::read(&second)?, b"merged");
            }
            ("missing", Some(OperationEvent::Restored { .. })) => {
                assert_eq!(fs::read(&second)?, b"second");
                assert!(staged_second.is_err());
            }
            (case, last) => panic!("{case}: {last:?}"),
        }
    }
    Ok(())
}
