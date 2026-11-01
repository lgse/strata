// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn moves_compare_inode_numbers_only_where_the_source_keeps_them() -> Result<(), Box<dyn Error>> {
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
    let _serial = serial()?;
    let mounted = |fs_type, options| {
        move || Quirks {
            mount_type: Some((fs_type, options)),
            ..Quirks::default()
        }
    };
    let share = |preset: Preset, no_birth_time| {
        move || Quirks {
            mount_type: preset().mount_type,
            no_birth_time,
            ..Quirks::default()
        }
    };
    type MakeQuirks = Box<dyn Fn() -> Quirks>;
    let cases: [(&str, MakeQuirks, bool, Meanwhile, Option<&str>); 12] = [
        (
            "sshfs",
            Box::new(Quirks::sshfs),
            false,
            Meanwhile::Renumbered,
            None,
        ),
        (
            "cifs noserverino",
            Box::new(share(Quirks::cifs_noserverino, false)),
            false,
            Meanwhile::Renumbered,
            None,
        ),
        (
            "cifs",
            Box::new(share(Quirks::cifs, false)),
            false,
            Meanwhile::Renumbered,
            Some("folder"),
        ),
        (
            "cifs without creation times",
            Box::new(share(Quirks::cifs, true)),
            false,
            Meanwhile::Renumbered,
            Some("folder"),
        ),
        (
            "cifs, server inode numbers turned off",
            Box::new(share(Quirks::cifs, false)),
            true,
            Meanwhile::Renumbered,
            None,
        ),
        (
            "cifs without creation times, server inode numbers turned off",
            Box::new(share(Quirks::cifs, true)),
            true,
            Meanwhile::Renumbered,
            None,
        ),
        (
            "vfat",
            Box::new(mounted("vfat", "rw,flush")),
            false,
            Meanwhile::Renumbered,
            None,
        ),
        (
            "cifs, a linked file replaced",
            Box::new(share(Quirks::cifs, false)),
            false,
            Meanwhile::ReplacedAlike {
                linked: Linked::Outside,
            },
            Some("folder/a.txt"),
        ),
        (
            "cifs, a file replaced once server inode numbers went",
            Box::new(share(Quirks::cifs, false)),
            true,
            Meanwhile::ReplacedAlike { linked: Linked::No },
            Some("folder/a.txt"),
        ),
        (
            "cifs, a linked file replaced once server inode numbers went",
            Box::new(share(Quirks::cifs, false)),
            true,
            Meanwhile::ReplacedAlike {
                linked: Linked::Outside,
            },
            Some("folder/a.txt"),
        ),
        (
            "cifs, a file linked to an item moved first replaced",
            Box::new(share(Quirks::cifs, false)),
            false,
            Meanwhile::ReplacedAlike {
                linked: Linked::SelectedFirst,
            },
            Some("folder/a.txt"),
        ),
        (
            "cifs, a file linked to an item moved first replaced once server inode numbers went",
            Box::new(share(Quirks::cifs, false)),
            true,
            Meanwhile::ReplacedAlike {
                linked: Linked::SelectedFirst,
            },
            Some("folder/a.txt"),
        ),
    ];
    for (case, quirks, turned_off, meanwhile, kept_at_source) in cases {
        let root = tempfile::tempdir()?;
        let source_dir = root.path().join("source");
        let folder = source_dir.join("folder");
        fs::create_dir_all(folder.join("sub"))?;
        fs::write(folder.join("a.txt"), b"a")?;
        fs::write(folder.join("sub/b.txt"), b"b")?;
        let (arrival, backup) = (root.path().join("arrival"), source_dir.join("backup.txt"));
        if let Meanwhile::ReplacedAlike { linked } = meanwhile {
            if linked != Linked::No && !hard_linked(&folder.join("a.txt"), &backup)? {
                eprintln!("skipped on a base without hard links: {case}");
                continue;
            }
            fs::write(&arrival, b"z")?;
            let hour_before = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)?
                .as_secs()
                - 3600;
            set_times(&folder.join("a.txt"), hour_before, 123_456_789)?;
            if turned_off && !a_later_change_time(&arrival, &folder.join("a.txt"), hour_before)? {
                eprintln!(
                    "skipped on a base that reports the modification time as the change time: {case}"
                );
                continue;
            }
            set_times(&arrival, hour_before, 123_456_789)?;
        }
        let evicted = [
            folder.clone(),
            folder.join("a.txt"),
            folder.join("sub"),
            folder.join("sub/b.txt"),
        ];
        let switched = Arc::new(AtomicBool::new(false));
        let (renumbering, meanwhile_at_source, meanwhile_at_destination) =
            match (meanwhile, turned_off) {
                (Meanwhile::Renumbered, false) => {
                    let switching = switched.clone();
                    let racer: super::super::fs_quirks::Racer = Box::new(move || {
                        for path in &evicted {
                            super::super::fs_quirks::renumbered(path);
                        }
                        switching.store(true, Ordering::Release);
                    });
                    (
                        vec![(Op::OpenDir, "folder", 1, racer)],
                        Vec::new(),
                        Vec::new(),
                    )
                }
                _ => {
                    let (at, switching) = (source_dir.clone(), switched.clone());
                    let (probe, file) = (root.path().join("probe"), folder.join("a.txt"));
                    let racer: super::super::fs_quirks::Racer = Box::new(move || {
                        let numbers_went =
                            !turned_off || super::super::fs_quirks::autodisable_serverino(&at);
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
                        switching.store(happened, Ordering::Release);
                    });
                    match meanwhile {
                        Meanwhile::ReplacedAlike {
                            linked: Linked::SelectedFirst,
                        } => (
                            Vec::new(),
                            vec![(Op::Unlink, "backup.txt", 0, racer)],
                            Vec::new(),
                        ),
                        _ => (
                            Vec::new(),
                            Vec::new(),
                            vec![(Op::Rename, "folder", 0, racer)],
                        ),
                    }
                }
            };
        let _source = FsQuirks::install(
            &source_dir,
            Quirks {
                before: renumbering,
                after: meanwhile_at_source,
                ..quirks()
            },
        );
        let destination = root.path().join("destination");
        fs::create_dir_all(&destination)?;
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                after: meanwhile_at_destination,
                ..Quirks::default()
            },
        );

        let selected_first = matches!(
            meanwhile,
            Meanwhile::ReplacedAlike {
                linked: Linked::SelectedFirst
            }
        );
        let sources = if selected_first {
            vec![backup.as_path(), &folder]
        } else {
            vec![folder.as_path()]
        };
        let (_operation, events) =
            paste_moves(95, &destination, &sources, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{case}: {:?}",
            events.borrow()
        );
        assert!(switched.load(Ordering::Acquire), "{case}");
        if selected_first {
            assert_eq!(fs::read(destination.join("backup.txt"))?, b"a", "{case}");
            assert!(!backup.exists(), "{case}");
        }
        assert_eq!(fs::read(destination.join("folder/a.txt"))?, b"a", "{case}");
        assert_eq!(
            fs::read(destination.join("folder/sub/b.txt"))?,
            b"b",
            "{case}"
        );
        if let Meanwhile::ReplacedAlike { .. } = meanwhile {
            assert_eq!(
                fs::read(folder.join("a.txt")).ok(),
                Some(b"z".to_vec()),
                "{case}"
            );
        }
        assert_eq!(folder.exists(), kept_at_source.is_some(), "{case}");
        let expected: Vec<_> = kept_at_source
            .map(|path| kept(source_dir.join(path), KeptCause::Changed))
            .into_iter()
            .collect();
        assert_eq!(reported(&events).kept, expected, "{case}");
    }
    Ok(())
}

#[test]
fn a_move_across_volumes_reads_the_source_mount_once() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir_all(&sources)?;
    fs::create_dir(&destination)?;
    let names = ["a.txt", "b.txt", "c.txt"];
    for name in names {
        fs::write(sources.join(name), name)?;
    }
    let (source_scope, _destination_scope) = separate_volumes(&sources, &destination);

    run_paste(PasteRequest {
        id: OperationRequestId(96),
        destination: Location::local(&destination),
        items: names
            .iter()
            .map(|name| PasteItem {
                source: Location::local(sources.join(name)),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            })
            .collect(),
        move_sources: true,
    });

    assert_eq!(super::super::names(&destination)?, names);
    assert!(super::super::names(&sources)?.is_empty());
    assert_eq!(source_scope.count(Op::MountType), 1);
    let smb = mount_capabilities(source_scope.key()).family == Some(Family::Smb);
    let reads_again = source_scope.count(Op::CurrentMountType);
    assert!(
        reads_again <= usize::from(smb) * names.len(),
        "{reads_again} reads of the current mount type, SMB: {smb}"
    );
    Ok(())
}

#[test]
fn a_source_entry_that_cannot_be_deleted_is_kept_with_the_reason() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let folder = source_dir.join("folder");
    fs::create_dir_all(folder.join("read-only"))?;
    fs::write(folder.join("a.txt"), b"a")?;
    let child = folder.join("read-only/child.txt");
    fs::write(&child, b"child")?;
    // As in a folder whose mode is 0555, for any user but root.
    let _source = FsQuirks::install(
        &source_dir,
        Quirks {
            fail: vec![(Op::Unlink, "child.txt", 0, Errno::ACCESS)],
            ..Quirks::default()
        },
    );
    let destination = root.path().join("destination");
    fs::create_dir_all(&destination)?;
    let _destination = FsQuirks::install(&destination, Quirks::default());

    let (_operation, events) =
        paste_move(96, &destination, &folder, TransferConflict::FailIfExists);
    pump_until_transfer(&events);

    assert!(matches!(
        terminal_transfer(&events.borrow()),
        Some(OperationEvent::Pasted { .. })
    ));
    assert_eq!(
        fs::read(destination.join("folder/read-only/child.txt"))?,
        b"child"
    );
    assert_eq!(names(&folder)?, ["read-only"]);
    assert_eq!(fs::read(&child)?, b"child");
    let reason = Errno::ACCESS.raw_os_error();
    assert_eq!(
        reported(&events).kept,
        [kept(child, KeptCause::Undeletable(reason))]
    );
    Ok(())
}
