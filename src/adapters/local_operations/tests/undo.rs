// SPDX-License-Identifier: MIT

use rustix::io::Errno;

use super::fs_quirks::{Op, Preset, Support, skips_malformed_name_on_fuse};
use super::*;
use crate::services::RenameRequest;

#[test]
fn undoing_a_move_returns_each_item_to_its_original_directory() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for quirks in [Quirks::default, Quirks::sshfs, Quirks::vboxsf] {
        let root = tempfile::tempdir()?;
        let _scope = FsQuirks::install(root.path(), quirks());
        let origin = root.path().join("origin");
        let archive = root.path().join("archive");
        fs::create_dir_all(&origin)?;
        fs::create_dir_all(&archive)?;
        let moved = archive.join("report.txt");
        fs::write(&moved, b"contents")?;
        let original = inode(&moved);

        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let _operation = LocalOperationProvider.undo_move(
            UndoMoveRequest {
                names: NameMode::Keep,
                cleanup_locations: Vec::new(),
                id: OperationRequestId(40),
                items: vec![UndoMoveItem {
                    record: MoveRecord {
                        original: Location::local(origin.join("report.txt")),
                        current: Location::local(&moved),
                        renamed: Vec::new(),
                    },
                    conflict: TransferConflict::FailIfExists,
                }],
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );

        drive_until_transfer_settles(&events);

        assert!(matches!(
            events.borrow().last(),
            Some(OperationEvent::Pasted { .. })
        ));
        assert!(names(&archive)?.is_empty());
        assert_eq!(names(&origin)?, ["report.txt"]);
        assert_eq!(inode(&origin.join("report.txt")), original);
        assert_eq!(fs::read(origin.join("report.txt"))?, b"contents");
    }
    Ok(())
}

#[test]
fn undoing_a_move_stops_at_an_unconfirmed_conflict() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let origin = root.path().join("origin");
    let archive = root.path().join("archive");
    fs::create_dir_all(&origin)?;
    fs::create_dir_all(&archive)?;
    let blocked = origin.join("report.txt");
    fs::write(&blocked, b"newer")?;
    let first = archive.join("notes.txt");
    let second = archive.join("report.txt");
    fs::write(&first, b"first")?;
    fs::write(&second, b"second")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.undo_move(
        UndoMoveRequest {
            names: NameMode::Keep,
            cleanup_locations: Vec::new(),
            id: OperationRequestId(41),
            items: vec![
                UndoMoveItem {
                    record: MoveRecord {
                        original: Location::local(origin.join("notes.txt")),
                        current: Location::local(&first),
                        renamed: Vec::new(),
                    },
                    conflict: TransferConflict::FailIfExists,
                },
                UndoMoveItem {
                    record: MoveRecord {
                        original: Location::local(&blocked),
                        current: Location::local(&second),
                        renamed: Vec::new(),
                    },
                    conflict: TransferConflict::FailIfExists,
                },
            ],
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    drive_until_transfer_settles(&events);

    let completed = match events.borrow().last() {
        Some(OperationEvent::TransferFailed {
            completed_locations,
            ..
        }) => completed_locations.clone(),
        other => panic!("expected a transfer failure, got {other:?}"),
    };
    assert_eq!(completed, vec![Location::local(&first)]);
    assert_eq!(
        item_moves(&events.borrow()),
        [(
            Location::local(&first),
            Location::local(origin.join("notes.txt"))
        )]
    );
    assert_eq!(fs::read(&blocked)?, b"newer");
    assert_eq!(fs::read(&second)?, b"second");
    assert_eq!(fs::read(origin.join("notes.txt"))?, b"first");
    Ok(())
}

#[test]
fn a_confirmed_undo_conflict_replaces_the_newer_item() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let origin = root.path().join("origin");
    let archive = root.path().join("archive");
    fs::create_dir_all(&origin)?;
    fs::create_dir_all(&archive)?;
    let original = origin.join("report.txt");
    let moved = archive.join("report.txt");
    fs::write(&original, b"newer")?;
    fs::write(&moved, b"moved")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.undo_move(
        UndoMoveRequest {
            names: NameMode::Keep,
            cleanup_locations: Vec::new(),
            id: OperationRequestId(42),
            items: vec![UndoMoveItem {
                record: MoveRecord {
                    original: Location::local(&original),
                    current: Location::local(&moved),
                    renamed: Vec::new(),
                },
                conflict: TransferConflict::ReplaceExisting,
            }],
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    drive_until_transfer_settles(&events);

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(!moved.exists());
    assert_eq!(fs::read(&original)?, b"moved");
    Ok(())
}

#[test]
fn an_undone_move_gives_names_back_as_they_were_and_a_redone_one_fixes_them()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let malformed = || OsString::from_vec(b"x\xc0\xafy".to_vec());
    type Row<'a> = (&'a str, Preset, NameMode, Vec<OsString>, [&'a str; 2]);
    let rows: [Row<'_>; 4] = [
        (
            "ntfs3 undo",
            Quirks::ntfs3,
            NameMode::Keep,
            vec!["a:b".into(), "CON".into()],
            ["CON", "a:b"],
        ),
        (
            "ntfs3 redo",
            Quirks::ntfs3,
            NameMode::Fix,
            vec!["a:b".into(), "CON".into()],
            ["CON_", "a_b"],
        ),
        // Whatever the mode, exfat-fuse never gets a name it would corrupt.
        (
            "exfat-fuse undo",
            Quirks::exfat_fuse,
            NameMode::Keep,
            vec![malformed(), "CON".into()],
            ["CON", "x__y"],
        ),
        (
            "exfat-fuse redo",
            Quirks::exfat_fuse,
            NameMode::Fix,
            vec![malformed(), "CON".into()],
            ["CON_", "x__y"],
        ),
    ];
    for (row, preset, mode, moved_names, expected) in rows {
        let root = tempfile::tempdir()?;
        if moved_names
            .iter()
            .any(|name| skips_malformed_name_on_fuse(root.path(), name))
        {
            continue;
        }
        let disk = root.path().join("disk");
        let moved = root.path().join("moved").join("project");
        fs::create_dir(&disk)?;
        fs::create_dir_all(&moved)?;
        for name in &moved_names {
            fs::write(moved.join(name), b"contents")?;
        }
        let _disk = FsQuirks::install(&disk, preset());

        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let _operation = LocalOperationProvider.undo_move(
            UndoMoveRequest {
                names: mode,
                cleanup_locations: Vec::new(),
                id: OperationRequestId(1530),
                items: vec![UndoMoveItem {
                    record: MoveRecord {
                        original: Location::local(disk.join("project")),
                        current: Location::local(&moved),
                        renamed: Vec::new(),
                    },
                    conflict: TransferConflict::FailIfExists,
                }],
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );
        drive_until_transfer_settles(&events);

        assert!(
            matches!(events.borrow().last(), Some(OperationEvent::Pasted { .. })),
            "{row}: {:?}",
            events.borrow()
        );
        assert_eq!(names(&disk.join("project"))?, expected, "{row}");
        assert!(!moved.exists(), "{row}");
    }
    Ok(())
}

fn refuses_a_colon_b(name: &OsStr) -> Option<Errno> {
    (name == "a:b").then_some(Errno::INVAL)
}

#[test]
fn rules_a_mount_learned_fix_names_a_redo_brings_but_not_those_an_undo_gives_back()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (mode, expected) in [(NameMode::Keep, "c:d"), (NameMode::Fix, "c_d")] {
        let root = tempfile::tempdir()?;
        let disk = root.path().join("disk");
        let source = root.path().join("source");
        let moved = root.path().join("moved").join("project");
        for folder in [&disk, &source, &moved] {
            fs::create_dir_all(folder)?;
        }
        fs::write(source.join("a:b"), b"refused")?;
        fs::write(moved.join("c:d"), b"held before the move")?;
        let _disk = FsQuirks::install(
            &disk,
            Quirks {
                mount_type: Some(("fuse.strictfs", "rw,user_id=0,group_id=0")),
                invalid_name: Some(refuses_a_colon_b),
                ..Quirks::default()
            },
        );
        run_paste(PasteRequest {
            id: OperationRequestId(1590),
            destination: Location::local(&disk),
            items: vec![PasteItem {
                source: Location::local(source.join("a:b")),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            }],
            move_sources: false,
        });
        assert_eq!(names(&disk)?, ["a_b"], "{mode:?}");

        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let _operation = LocalOperationProvider.undo_move(
            UndoMoveRequest {
                names: mode,
                cleanup_locations: Vec::new(),
                id: OperationRequestId(1591),
                items: vec![UndoMoveItem {
                    record: MoveRecord {
                        original: Location::local(disk.join("project")),
                        current: Location::local(&moved),
                        renamed: Vec::new(),
                    },
                    conflict: TransferConflict::FailIfExists,
                }],
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );
        drive_until_transfer_settles(&events);

        assert!(
            matches!(events.borrow().last(), Some(OperationEvent::Pasted { .. })),
            "{mode:?}: {:?}",
            events.borrow()
        );
        assert_eq!(names(&disk.join("project"))?, [expected], "{mode:?}");
    }
    Ok(())
}

#[test]
fn rename_undo_restores_a_native_name_byte_for_byte() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let original_parent = root.path().join("original");
    let current_parent = root.path().join("current");
    fs::create_dir_all(&original_parent)?;
    fs::create_dir_all(&current_parent)?;
    let original_name = OsString::from_vec(b"original-\xff.txt".to_vec());
    let original = original_parent.join(&original_name);
    let current = current_parent.join("renamed.txt");
    fs::write(&current, b"contents")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.undo_rename(
        UndoRenameRequest {
            id: OperationRequestId(43),
            current: Location::local(&current),
            original: Location::local(&original),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    drive_until_rename_settles(&events);

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Renamed { .. })
    ));
    assert!(!current.exists());
    assert_eq!(fs::read(&original)?, b"contents");
    assert_eq!(
        original_name.as_encoded_bytes(),
        original
            .file_name()
            .expect("original test path should have a file name")
            .as_encoded_bytes()
    );
    Ok(())
}

#[test]
fn rename_undo_refuses_an_occupied_destination_and_can_retry() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let original_parent = root.path().join("original");
    let current_parent = root.path().join("current");
    fs::create_dir_all(&original_parent)?;
    fs::create_dir_all(&current_parent)?;
    let original = original_parent.join("item.txt");
    let current = current_parent.join("renamed.txt");
    fs::write(&original, b"occupant")?;
    fs::write(&current, b"renamed")?;
    let request = || UndoRenameRequest {
        id: OperationRequestId(44),
        current: Location::local(&current),
        original: Location::local(&original),
    };

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    {
        let _operation = LocalOperationProvider.undo_rename(
            request(),
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );
        drive_until_rename_settles(&events);
    }
    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Failed { .. })
    ));
    assert_eq!(fs::read(&original)?, b"occupant");
    assert_eq!(fs::read(&current)?, b"renamed");

    fs::remove_file(&original)?;
    events.borrow_mut().clear();
    let emitted = events.clone();
    let _operation = LocalOperationProvider.undo_rename(
        request(),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    drive_until_rename_settles(&events);
    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Renamed { .. })
    ));
    assert!(!current.exists());
    assert_eq!(fs::read(&original)?, b"renamed");
    Ok(())
}

fn inode(path: &Path) -> u64 {
    std::os::unix::fs::MetadataExt::ino(&fs::symlink_metadata(path).expect("entry exists"))
}

fn undo_rename(current: &Path, original: &Path) -> Vec<OperationEvent> {
    let request = UndoRenameRequest {
        id: OperationRequestId(47),
        current: Location::local(current),
        original: Location::local(original),
    };
    settle(|emit| LocalOperationProvider.undo_rename(request, emit))
}

fn failure(events: &[OperationEvent]) -> String {
    match events {
        [OperationEvent::Failed { message, .. }] => message.clone(),
        other => panic!("expected one failure, got {other:?}"),
    }
}

#[test]
fn renames_and_their_undo_keep_the_item_where_rename_flags_or_links_are_missing()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let linked_before_a_probe = || Quirks {
        rename_flags: Support::Rejected(Errno::NOSYS),
        ..Quirks::sshfs()
    };
    let ntfs_3g = || Quirks {
        rename_flags: Support::Rejected(Errno::INVAL),
        ..Quirks::ntfs_3g()
    };
    let cut = "x".repeat(240);
    let cases: [(Preset, &str); 5] = [
        (Quirks::without_rename_flags, "renamed"),
        (Quirks::vboxsf, "renamed"),
        (Quirks::sshfs, "renamed"),
        (linked_before_a_probe, "renamed"),
        (ntfs_3g, &cut),
    ];
    for (quirks, name) in cases {
        for folder in [false, true] {
            let root = tempfile::tempdir()?;
            let _scope = FsQuirks::install(root.path(), quirks());
            let source = root.path().join("item");
            let renamed = root.path().join(name);
            if folder {
                fs::create_dir(&source)?;
            } else {
                fs::write(&source, b"data")?;
            }
            let original = inode(&source);

            let events = rename(&source, name);
            assert!(
                matches!(events.as_slice(), [OperationEvent::Renamed { .. }]),
                "{events:?}"
            );
            assert_eq!(inode(&renamed), original);

            let events = undo_rename(&renamed, &source);
            assert!(
                matches!(events.as_slice(), [OperationEvent::Renamed { .. }]),
                "{events:?}"
            );
            assert_eq!(inode(&source), original);
            assert_eq!(names(root.path())?, ["item"]);
        }
    }
    Ok(())
}

#[test]
fn a_typed_name_the_location_would_store_otherwise_is_refused_before_the_rename()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let characters = |characters: &[&str]| {
        rust_i18n::t!(
            "Names in this location can’t contain %{characters}",
            characters = crate::i18n::list(
                characters
                    .iter()
                    .map(|character| crate::i18n::quoted(character))
            )
        )
        .into_owned()
    };
    let reserved = |name: &str| {
        rust_i18n::t!("“%{name}” is a reserved name in this location", name = name).into_owned()
    };
    let gvfs_bridge: Preset = || Quirks {
        mount_type: Some(("fuse.gvfsd-fuse", "rw,user_id=1000,group_id=1000")),
        ..Quirks::default()
    };
    let long_folder_name = "f".repeat(214);
    let rows: [(Preset, &str, bool, &str, Option<String>); 6] = [
        (
            Quirks::vfat,
            "",
            false,
            "a:b?",
            Some(characters(&[":", "?"])),
        ),
        (
            Quirks::vfat,
            "",
            false,
            "notes.",
            Some(crate::i18n::tr(
                "Names in this location can’t end with a space or a period",
            )),
        ),
        (Quirks::vfat, "", true, "CON", Some(reserved("CON"))),
        // Samba makes only new folders under longer temporary names.
        (Quirks::cifs, "", true, &long_folder_name, None),
        (Quirks::ext4, "", false, "a:b.", None),
        (
            gvfs_bridge,
            "mtp:host=Phone",
            false,
            " x",
            Some(crate::i18n::tr(
                "Names in this location can’t start with a space",
            )),
        ),
    ];
    for (quirks, backend, folder, typed, refusal) in rows {
        let root = tempfile::tempdir()?;
        let mount = root.path().join("mount");
        let parent = mount.join(backend);
        let item = parent.join("item");
        fs::create_dir_all(&parent)?;
        if folder {
            fs::create_dir(&item)?;
        } else {
            fs::write(&item, b"data")?;
        }
        let _scope = FsQuirks::install(&mount, quirks());
        let entry = if folder {
            directory_entry(&item)
        } else {
            file_entry(&item)
        };

        let events = settle(|emit| {
            LocalOperationProvider.rename(
                RenameRequest {
                    id: OperationRequestId(48),
                    entry,
                    new_name: typed.to_owned(),
                },
                emit,
            )
        });

        match refusal {
            Some(message) => {
                assert_eq!(failure(&events), message, "{typed:?}");
                assert_eq!(names(&parent)?, ["item"], "{typed:?}");
            }
            None => {
                assert!(
                    matches!(events.as_slice(), [OperationEvent::Renamed { .. }]),
                    "{typed:?}: {events:?}"
                );
                assert_eq!(names(&parent)?, [typed], "{typed:?}");
            }
        }
    }
    Ok(())
}

#[test]
fn a_typed_name_at_a_gvfs_address_is_refused_by_the_rules_of_its_scheme()
-> Result<(), Box<dyn Error>> {
    use super::super::typed_names::refused_in;
    let _serial = serial()?;
    let colon = rust_i18n::t!(
        "Names in this location can’t contain %{characters}",
        characters = crate::i18n::quoted(":")
    )
    .into_owned();
    let leading_space = crate::i18n::tr("Names in this location can’t start with a space");
    for (uri, typed, refusal) in [
        ("smb://server/share", "a:b", Some(colon)),
        ("mtp://Phone/Internal", " x", Some(leading_space)),
        ("sftp://host/home", "a:b", None),
    ] {
        let folder = gio::File::for_uri(uri);
        let refused = glib::MainContext::default().block_on(refused_in(&folder, typed, false));
        assert_eq!(refused, refusal, "{uri}");
    }
    Ok(())
}

#[test]
fn a_rename_gio_has_no_backend_for_fails_instead_of_ending_the_process()
-> Result<(), Box<dyn Error>> {
    use super::super::{NameKept, restore_name};
    let _serial = serial()?;
    let uri = "smb://server/share/item.txt";
    assert_eq!(
        gio::File::for_uri(uri).type_().name(),
        "GDummyFile",
        "needs GIO without GVfs (GIO_USE_VFS=local), whose stand-in file cannot rename"
    );
    let unsupported = crate::services::backend_unavailable_message(uri);
    let entry = FileEntry {
        location: Location::uri(uri),
        ..file_entry(Path::new("/share/item.txt"))
    };

    let events = settle(|emit| {
        LocalOperationProvider.rename(
            RenameRequest {
                id: OperationRequestId(49),
                entry,
                new_name: "renamed.txt".to_owned(),
            },
            emit,
        )
    });
    let given_back = glib::MainContext::default().block_on(restore_name(
        &Location::uri(uri),
        &Location::uri("smb://server/share/item:1.txt"),
    ));

    assert_eq!(failure(&events), unsupported);
    assert!(
        matches!(&given_back, Err(NameKept::Failed(error))
            if *error == crate::services::error_detail(unsupported.clone())),
        "the name was not kept as unsupported"
    );
    Ok(())
}

#[test]
fn a_rename_never_replaces_an_item_created_meanwhile() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for quirks in [Quirks::default, Quirks::vboxsf] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("item.txt");
        let taken = root.path().join("new.txt");
        fs::write(&source, b"mine")?;
        let raced = taken.clone();
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                before: vec![(
                    Op::Rename,
                    "new.txt",
                    0,
                    Box::new(move || fs::write(&raced, b"theirs").expect("racer")),
                )],
                ..quirks()
            },
        );

        assert_eq!(
            failure(&rename(&source, "new.txt")),
            "\u{201c}new.txt\u{201d} already exists"
        );
        assert_eq!(fs::read(&source)?, b"mine");
        assert_eq!(fs::read(&taken)?, b"theirs");
    }
    Ok(())
}

#[test]
fn a_case_only_rename_and_its_undo_keep_the_item_or_say_where_it_was_left()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("report.txt");
    let renamed = root.path().join("Report.txt");
    fs::write(&source, b"data")?;
    let original = inode(&source);
    {
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                case_insensitive: true,
                ..Quirks::default()
            },
        );
        assert!(matches!(
            rename(&source, "Report.txt").as_slice(),
            [OperationEvent::Renamed { .. }]
        ));
        assert_eq!(inode(&renamed), original);
        assert!(matches!(
            undo_rename(&renamed, &source).as_slice(),
            [OperationEvent::Renamed { .. }]
        ));
        assert_eq!(inode(&source), original);
    }

    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            case_insensitive: true,
            // Skips the direct attempts, then fails the second step and the way back.
            fail: vec![
                (Op::Rename, "Report.txt", 1, Errno::IO),
                (Op::Rename, "report.txt", 1, Errno::IO),
            ],
            ..Quirks::default()
        },
    );
    let message = failure(&rename(&source, "Report.txt"));

    let left = names(root.path())?;
    let [originals] = left.as_slice() else {
        panic!("expected one entry, got {left:?}");
    };
    assert!(originals.starts_with(".strata-originals-"), "{originals}");
    assert_eq!(
        inode(&root.path().join(originals).join("report.txt")),
        original
    );
    assert_eq!(
        message,
        format!(
            "Could not rename \u{201c}report.txt\u{201d}; it was left as \u{201c}{originals}/report.txt\u{201d}: A device input/output error occurred"
        )
    );
    Ok(())
}

fn undo_items(
    id: u64,
    moved: &[(&Path, &Path)],
    conflict: TransferConflict,
) -> (LoadHandle, Rc<RefCell<Vec<OperationEvent>>>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = LocalOperationProvider.undo_move(
        UndoMoveRequest {
            names: NameMode::Keep,
            cleanup_locations: Vec::new(),
            id: OperationRequestId(id),
            items: moved
                .iter()
                .map(|(original, current)| UndoMoveItem {
                    record: MoveRecord {
                        original: Location::local(original),
                        current: Location::local(current),
                        renamed: Vec::new(),
                    },
                    conflict,
                })
                .collect(),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    (operation, events)
}

#[test]
fn undoing_a_cross_volume_move_deletes_the_moved_items_once_they_are_flushed()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let names = ["notes.txt", "photo.jpg", "report.txt"];
    for (id, flushed) in [(97, true), (98, false)] {
        let root = tempfile::tempdir()?;
        let origin = root.path().join("usb");
        let archive = root.path().join("archive");
        fs::create_dir_all(&origin)?;
        fs::create_dir_all(&archive)?;
        let (originals, currents): (Vec<PathBuf>, Vec<PathBuf>) = names
            .iter()
            .map(|name| (origin.join(name), archive.join(name)))
            .unzip();
        for current in &currents {
            fs::write(current, b"contents")?;
        }
        // The items share one barrier, before which every moved item stays.
        let during = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (record, watched) = (during.clone(), currents.clone());
        let racer: super::fs_quirks::Racer = Box::new(move || {
            record
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(watched.iter().all(|current| current.exists()));
        });
        let _archive = FsQuirks::install(&archive, Quirks::default());
        let _origin = FsQuirks::install(
            &origin,
            Quirks {
                before: vec![(Op::SyncFs, "*", 0, racer)],
                fail: if flushed {
                    Vec::new()
                } else {
                    vec![(Op::SyncFs, "*", 0, Errno::IO)]
                },
                ..Quirks::ext4()
            },
        );

        let moved: Vec<(&Path, &Path)> = originals
            .iter()
            .zip(&currents)
            .map(|(original, current)| (original.as_path(), current.as_path()))
            .collect();
        let (_operation, events) = undo_items(id, &moved, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        assert_eq!(
            *during.lock().unwrap_or_else(|error| error.into_inner()),
            [true],
            "flushed: {flushed}"
        );
        for original in &originals {
            assert_eq!(fs::read(original)?, b"contents");
        }
        if flushed {
            assert!(matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ));
            assert!(currents.iter().all(|current| !current.exists()));
            continue;
        }
        assert!(matches!(terminal_transfer(&events.borrow()),
            Some(OperationEvent::TransferFailed { message, .. })
            if message.contains("writes may still be pending")));
        for current in &currents {
            assert_eq!(fs::read(current)?, b"contents");
        }
    }
    Ok(())
}

/// Moves `items` onto a fresh mount, lands the deferred closes, undoes the
/// move `undone_after` later, and counts the forced stats of each `counted`
/// entry during the undo. With `read_after`, the items are copied away that
/// long after the move first, and the closes land after that read.
fn forced_stats_undoing_a_move(
    preset: Preset,
    (items, counted): (&[&str], &[&str]),
    (started, read_after, undone_after): (SystemTime, Option<Duration>, Duration),
) -> Result<Vec<usize>, Box<dyn Error>> {
    let row = format!(
        "{:?} {items:?} read after {read_after:?}, undone after {undone_after:?}",
        preset().mount_type.map(|(kind, _)| kind)
    );
    let _clock = ClockGuard::set(started);
    let root = tempfile::tempdir()?;
    let origin = root.path().join("origin");
    let share = root.path().join("share");
    let viewer = root.path().join("viewer");
    fs::create_dir_all(origin.join("project/sub"))?;
    fs::create_dir(&share)?;
    fs::create_dir(&viewer)?;
    fs::write(origin.join("notes.txt"), b"notes")?;
    fs::write(origin.join("project/sub/b.txt"), b"b")?;
    for folder in [origin.clone(), origin.join("project")] {
        std::os::unix::fs::symlink("elsewhere", folder.join("link"))?;
        rustix::fs::mknodat(
            rustix::fs::CWD,
            folder.join("pipe"),
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::from_raw_mode(0o600),
            0,
        )?;
    }
    let scope = FsQuirks::install(&share, preset());
    let request = |id, destination: &Path, folder: &Path, move_sources| PasteRequest {
        id: OperationRequestId(id),
        destination: Location::local(destination),
        items: items
            .iter()
            .map(|item| PasteItem {
                source: Location::local(folder.join(item)),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            })
            .collect(),
        move_sources,
    };
    run_paste(request(1592, &share, &origin, true));
    if let Some(read_after) = read_after {
        super::super::copy_engine::set_clock_for_test(Some(started + read_after));
        run_paste(request(1593, &viewer, &share, false));
    }
    land_deferred_closes(&share)?;
    super::super::copy_engine::set_clock_for_test(Some(started + undone_after));
    scope.clear_calls();

    let moved = items
        .iter()
        .map(|item| (origin.join(item), share.join(item)))
        .collect::<Vec<_>>();
    let moved: Vec<(&Path, &Path)> = moved
        .iter()
        .map(|(original, current)| (original.as_path(), current.as_path()))
        .collect();
    let (_operation, events) = undo_items(1594, &moved, TransferConflict::FailIfExists);
    pump_until_transfer(&events);

    let events = events.take();
    assert!(
        matches!(
            terminal_transfer(&events),
            Some(OperationEvent::Pasted { .. })
        ),
        "{row}: {events:?}"
    );
    assert_eq!(followed_report(&events).kept, [], "{row}");
    assert!(names(&share)?.is_empty(), "{row}");
    for item in items {
        assert!(
            origin.join(item).symlink_metadata().is_ok(),
            "{row}: {item}"
        );
    }
    let calls = scope.calls();
    Ok(counted
        .iter()
        .map(|name| {
            calls
                .iter()
                .filter(|call| call.op == Op::Stat && call.forced)
                .filter(|call| call.names.iter().any(|named| named == name))
                .count()
        })
        .collect())
}

#[test]
fn undoing_a_move_onto_a_share_forces_each_stat_it_records_there_and_nowhere_else()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let started = SystemTime::now();
    let (now, later) = (Duration::ZERO, Duration::from_millis(1500));
    let in_folder = ["b.txt", "link", "pipe"];
    // The last row reads the items once the cache from the move expired, and
    // their deferred closes reach the server only after that.
    let rows: [(&[&str], &[&str], _, _); 5] = [
        (&["notes.txt"], &["notes.txt"], None, now),
        (&["project"], &in_folder, None, now),
        (&["link", "pipe"], &["link", "pipe"], None, now),
        (&["notes.txt", "project"], &in_folder, None, later),
        (
            &["notes.txt", "project"],
            &["notes.txt", "b.txt"],
            Some(Duration::from_millis(1100)),
            Duration::from_millis(1200),
        ),
    ];
    for (items, counted, read_after, undone_after) in rows {
        let timing = (started, read_after, undone_after);
        let on_ext4 = forced_stats_undoing_a_move(Quirks::ext4, (items, counted), timing)?;
        let on_nfs = forced_stats_undoing_a_move(Quirks::nfs42, (items, counted), timing)?;
        let on_share = forced_stats_undoing_a_move(Quirks::cifs_posix, (items, counted), timing)?;

        let context = format!("{items:?} read after {read_after:?}");
        assert_eq!(on_nfs, on_ext4, "{context}");
        let recorded_forced = on_ext4.iter().map(|count| count + 1).collect::<Vec<_>>();
        assert_eq!(on_share, recorded_forced, "{context}");
    }
    Ok(())
}

#[test]
fn a_cancel_that_cuts_off_a_move_back_counts_it_as_not_attempted() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let originals = root.path().join("originals");
    let moved = root.path().join("moved");
    fs::create_dir(&originals)?;
    fs::create_dir(&moved)?;
    let names = ["big.bin", "next.bin"];
    for name in names {
        fs::write(moved.join(name), vec![0x5a; 4 * 1024 * 1024])?;
    }
    let cancel = CancelFromMain::default();
    let _moved = FsQuirks::install(&moved, Quirks::default());
    let _originals = FsQuirks::install(
        &originals,
        Quirks {
            before: vec![
                (Op::CopyRange, "*", 0, cancel.racer()),
                (Op::Write, "*", 0, cancel.racer()),
            ],
            reflink: Support::Rejected(Errno::OPNOTSUPP),
            ..Quirks::default()
        },
    );
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let mut operation = Some(
        LocalOperationProvider.undo_move(
            UndoMoveRequest {
                names: NameMode::Keep,
                id: OperationRequestId(1615),
                items: names
                    .map(|name| UndoMoveItem {
                        record: MoveRecord {
                            original: Location::local(originals.join(name)),
                            current: Location::local(moved.join(name)),
                            renamed: Vec::new(),
                        },
                        conflict: TransferConflict::FailIfExists,
                    })
                    .into(),
                cleanup_locations: Vec::new(),
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        ),
    );

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
        assert!(Instant::now() < deadline, "the move back never ended");
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
    assert!(result.failed.is_empty());
    assert_eq!(
        result.not_attempted,
        names.map(|name| Location::local(moved.join(name)))
    );
    assert_eq!(fs::read_dir(&originals)?.count(), 0);
    for name in names {
        assert_eq!(fs::read(moved.join(name))?.len(), 4 * 1024 * 1024);
    }
    Ok(())
}

#[test]
fn cancelled_rename_undo_reports_cancellation_without_moving_the_item() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let original_parent = root.path().join("original");
    let current_parent = root.path().join("current");
    fs::create_dir_all(&original_parent)?;
    fs::create_dir_all(&current_parent)?;
    let original = original_parent.join("item.txt");
    let current = current_parent.join("renamed.txt");
    fs::write(&current, b"renamed")?;
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = LocalOperationProvider.undo_rename(
        UndoRenameRequest {
            id: OperationRequestId(45),
            current: Location::local(&current),
            original: Location::local(&original),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    drop(operation);
    drive_until_rename_settles(&events);

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Cancelled { .. })
    ));
    assert!(current.exists());
    assert!(!original.exists());
    Ok(())
}

#[test]
fn undoing_a_grouping_reports_the_trashed_group_folder() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::undo::undoing_a_grouping_reports_the_trashed_group_folder",
        || {
            let root = tempfile::tempdir().expect("root");
            let group = root.path().join("group");
            fs::create_dir(&group).expect("group folder");
            fs::write(group.join("report.txt"), b"contents").expect("grouped item");
            let identity = numbers_at(&group);
            let events = Rc::new(RefCell::new(Vec::new()));
            let emitted = events.clone();
            let _operation = LocalOperationProvider.undo_move(
                UndoMoveRequest {
                    names: NameMode::Keep,
                    cleanup_locations: vec![Location::local(&group)],
                    id: OperationRequestId(42),
                    items: vec![UndoMoveItem {
                        record: MoveRecord {
                            original: Location::local(root.path().join("report.txt")),
                            current: Location::local(group.join("report.txt")),
                            renamed: Vec::new(),
                        },
                        conflict: TransferConflict::FailIfExists,
                    }],
                },
                Rc::new(move |event| emitted.borrow_mut().push(event)),
            );

            drive_until_transfer_settles(&events);

            let events = events.borrow();
            assert!(matches!(events.last(), Some(OperationEvent::Pasted { .. })));
            assert!(!group.exists());
            assert_eq!(
                trashed_items(&events),
                [(Location::local(&group), identity)]
            );
        },
    );
}

#[test]
fn undoing_a_grouping_without_trash_removes_only_an_empty_group_folder()
-> Result<(), Box<dyn Error>> {
    const NAME: &str = "adapters::local_operations::tests::undo::undoing_a_grouping_without_trash_removes_only_an_empty_group_folder";
    let _serial = serial()?;
    let Some((_home_side, volume)) = crate::test_support::distinct_device_dirs(NAME) else {
        return Ok(());
    };
    for arrived_meanwhile in [false, true] {
        let root = tempfile::TempDir::new_in(volume.path())?;
        let group = root.path().join("group");
        fs::create_dir(&group)?;
        fs::write(group.join("report.txt"), b"contents")?;
        if arrived_meanwhile {
            fs::write(group.join("arrived.txt"), b"new")?;
        }
        let can_trash = gio::File::for_path(&group)
            .query_info(
                "access::can-trash",
                gio::FileQueryInfoFlags::NONE,
                gio::Cancellable::NONE,
            )?
            .boolean("access::can-trash");
        assert!(!can_trash, "precondition: GIO refuses Trash on tmpfs");
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let _operation = LocalOperationProvider.undo_move(
            UndoMoveRequest {
                cleanup_locations: vec![Location::local(&group)],
                id: OperationRequestId(43),
                items: vec![UndoMoveItem {
                    record: MoveRecord {
                        original: Location::local(root.path().join("report.txt")),
                        current: Location::local(group.join("report.txt")),
                        renamed: Vec::new(),
                    },
                    conflict: TransferConflict::FailIfExists,
                }],
                names: NameMode::Keep,
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );
        drive_until_transfer_settles(&events);
        let events = events.borrow();
        assert!(
            matches!(events.last(), Some(OperationEvent::Pasted { .. })),
            "{events:?}"
        );
        assert!(trashed_items(&events).is_empty());
        assert_eq!(fs::read(root.path().join("report.txt"))?, b"contents");
        assert_eq!(group.exists(), arrived_meanwhile);
    }
    Ok(())
}

#[test]
fn undoing_a_move_gives_back_names_numbered_as_aliases() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for taken in [false, true] {
        let root = tempfile::tempdir()?;
        let origin = root.path().join("origin");
        let usb = root.path().join("usb");
        let project = origin.join("project");
        fs::create_dir_all(&project)?;
        fs::create_dir(&usb)?;
        fs::write(project.join("Makefile"), b"upper")?;
        fs::write(project.join("makefile"), b"lower")?;
        fs::create_dir(project.join("Docs"))?;
        fs::create_dir(project.join("docs"))?;
        fs::write(project.join("docs/Makefile"), b"inner upper")?;
        fs::write(project.join("docs/makefile"), b"inner lower")?;
        let restored = project.join("makefile");
        let racer: super::fs_quirks::Racer = {
            let restored = restored.clone();
            Box::new(move || fs::write(&restored, b"taken").expect("take the name"))
        };
        let _origin = FsQuirks::install(
            &origin,
            Quirks {
                before: taken
                    .then_some((Op::Rename, "makefile (1)", 0, racer))
                    .into_iter()
                    .collect(),
                ..Quirks::default()
            },
        );
        let _usb = FsQuirks::install(&usb, Quirks::vfat());
        let moved = run_paste(PasteRequest {
            id: OperationRequestId(44),
            destination: Location::local(&usb),
            items: vec![PasteItem {
                source: Location::local(&project),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            }],
            move_sources: true,
        });
        let renamed = moved
            .iter()
            .flat_map(|event| match event {
                OperationEvent::ItemsReported { report, .. } => report.renamed.clone(),
                _ => Vec::new(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            names(&usb.join("project"))?,
            ["Docs", "Makefile", "docs (1)", "makefile (1)"]
        );

        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let _operation = LocalOperationProvider.undo_move(
            UndoMoveRequest {
                names: NameMode::Keep,
                cleanup_locations: Vec::new(),
                id: OperationRequestId(45),
                items: vec![UndoMoveItem {
                    record: MoveRecord {
                        original: Location::local(&project),
                        current: Location::local(usb.join("project")),
                        renamed,
                    },
                    conflict: TransferConflict::FailIfExists,
                }],
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );
        drive_until_transfer_settles(&events);

        assert!(matches!(
            events.borrow().last(),
            Some(OperationEvent::Pasted { .. })
        ));
        assert_eq!(fs::read(project.join("Makefile"))?, b"upper");
        assert_eq!(fs::read(project.join("docs/makefile"))?, b"inner lower");
        assert_eq!(fs::read(project.join("docs/Makefile"))?, b"inner upper");
        let reported = events
            .borrow()
            .iter()
            .flat_map(|event| match event {
                OperationEvent::ItemsReported { report, .. } => report.renamed.clone(),
                _ => Vec::new(),
            })
            .collect::<Vec<_>>();
        if taken {
            assert_eq!(
                names(&project)?,
                ["Docs", "Makefile", "docs", "makefile", "makefile (1)"]
            );
            assert_eq!(fs::read(&restored)?, b"taken");
            assert_eq!(fs::read(project.join("makefile (1)"))?, b"lower");
            assert_eq!(
                reported,
                [crate::services::RenamedItem {
                    requested: Location::local(&restored),
                    saved_as: Location::local(project.join("makefile (1)")),
                    cause: crate::services::RenameCause::Taken,
                }]
            );
        } else {
            assert_eq!(names(&project)?, ["Docs", "Makefile", "docs", "makefile"]);
            assert_eq!(fs::read(&restored)?, b"lower");
            assert!(reported.is_empty());
        }
    }
    Ok(())
}

fn moved_onto_fat(project: &Path, usb: &Path) -> MoveRecord {
    let moved = run_paste(PasteRequest {
        id: OperationRequestId(1590),
        destination: Location::local(usb),
        items: vec![PasteItem {
            source: Location::local(project),
            conflict: TransferConflict::FailIfExists,
            target_name: None,
        }],
        move_sources: true,
    });
    let current = Location::local(usb.join("project"));
    MoveRecord {
        original: Location::local(project),
        renamed: followed_report(&moved)
            .renamed
            .into_iter()
            .filter(|item| item.saved_as != current && item.saved_as.is_within(&current))
            .collect(),
        current,
    }
}

fn undone(record: MoveRecord) -> Vec<OperationEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.undo_move(
        UndoMoveRequest {
            names: NameMode::Keep,
            cleanup_locations: Vec::new(),
            id: OperationRequestId(1591),
            items: vec![UndoMoveItem {
                record,
                conflict: TransferConflict::FailIfExists,
            }],
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    drive_until_transfer_settles(&events);
    assert!(
        matches!(events.borrow().last(), Some(OperationEvent::Pasted { .. })),
        "{:?}",
        events.borrow()
    );
    events.take()
}

fn refuses_refused(name: &OsStr) -> Option<Errno> {
    let name = name.as_encoded_bytes();
    if name.starts_with(b"refused:") {
        Some(Errno::INVAL)
    } else {
        name.starts_with(b"illegal:").then_some(Errno::ILSEQ)
    }
}

#[test]
fn undoing_a_move_gives_fixed_names_back_and_reports_those_it_cannot() -> Result<(), Box<dyn Error>>
{
    use crate::services::{KeptCause, RenameCause};
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let origin = root.path().join("origin");
    let usb = root.path().join("usb");
    let project = origin.join("project");
    fs::create_dir_all(project.join("x:y"))?;
    fs::create_dir(&usb)?;
    for name in [
        "x:y/a:b",
        "CON",
        "taken:1",
        "refused:2",
        "failed:3",
        "illegal:4",
        "plain.txt",
        "long",
    ] {
        fs::write(project.join(name), name)?;
    }
    let taker: super::fs_quirks::Racer = {
        let taken = project.join("taken:1");
        Box::new(move || fs::write(&taken, b"taken").expect("take the name"))
    };
    let _origin = FsQuirks::install(
        &origin,
        Quirks {
            invalid_name: Some(refuses_refused),
            fail: vec![(Op::Rename, "failed:3", 0, Errno::ACCESS)],
            before: vec![(Op::Rename, "taken:1", 0, taker)],
            ..Quirks::default()
        },
    );
    let _usb = FsQuirks::install(&usb, Quirks::vfat());
    let mut record = moved_onto_fat(&project, &usb);
    assert_eq!(
        names(&usb.join("project"))?,
        [
            "CON_",
            "failed_3",
            "illegal_4",
            "long",
            "plain.txt",
            "refused_2",
            "taken_1",
            "x_y"
        ]
    );
    // As a name too long for the original, cut on a mount that takes more.
    let long = "l".repeat(300);
    record.renamed.push(crate::services::RenamedItem {
        requested: Location::local(usb.join("project").join(&long)),
        saved_as: Location::local(usb.join("project/long")),
        cause: RenameCause::TooLong,
    });

    let report = followed_report(&undone(record));

    assert_eq!(
        names(&project)?,
        [
            "CON",
            "failed_3",
            "illegal_4",
            "long",
            "plain.txt",
            "refused_2",
            "taken:1",
            "taken_1",
            "x:y"
        ]
    );
    assert_eq!(fs::read(project.join("x:y/a:b"))?, b"x:y/a:b");
    assert_eq!(fs::read(project.join("taken_1"))?, b"taken:1");
    assert_eq!(fs::read(project.join("taken:1"))?, b"taken");
    assert_eq!(
        renamed_beneath(&project, &report),
        [
            ("illegal:4", "illegal_4", RenameCause::Invalid),
            (long.as_str(), "long", RenameCause::TooLong),
            ("refused:2", "refused_2", RenameCause::Invalid),
            ("taken:1", "taken_1", RenameCause::Taken),
        ]
        .map(|(name, saved_as, cause)| (name.to_owned(), saved_as.to_owned(), cause))
    );
    let denied = crate::services::error_detail(super::super::os_error_text(Errno::ACCESS));
    assert_eq!(
        report.kept,
        [crate::services::KeptItem {
            location: Location::local(project.join("failed:3")),
            cause: KeptCause::NameNotRestored {
                saved_as: Location::local(project.join("failed_3")),
                error: denied,
            },
        }]
    );
    Ok(())
}

#[test]
fn undo_says_nothing_of_fixed_entries_gone_from_the_names_they_were_saved_as()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // As a share's location, which GIO renames.
    for through_gio in [false, true] {
        let root = tempfile::tempdir()?;
        let origin = root.path().join("origin");
        let usb = root.path().join("usb");
        let project = origin.join("project");
        fs::create_dir_all(project.join("gone:4"))?;
        fs::create_dir(&usb)?;
        for name in ["deleted:1", "renamed:2", "kept:3", "gone:4/a:b"] {
            fs::write(project.join(name), name)?;
        }
        let _usb = FsQuirks::install(&usb, Quirks::vfat());
        let mut record = moved_onto_fat(&project, &usb);
        if through_gio {
            record.original = Location::uri(gio::File::for_path(&project).uri());
        }
        let moved = usb.join("project");
        fs::remove_file(moved.join("deleted_1"))?;
        fs::rename(moved.join("renamed_2"), moved.join("mine.txt"))?;
        fs::remove_dir_all(moved.join("gone_4"))?;

        let report = followed_report(&undone(record));

        assert_eq!(names(&project)?, ["kept:3", "mine.txt"], "{through_gio}");
        assert!(report.is_empty(), "{through_gio}: {report:?}");
    }
    Ok(())
}

#[test]
fn undoing_a_move_back_to_another_kind_of_location_gives_fixed_names_back()
-> Result<(), Box<dyn Error>> {
    use crate::services::{RenameCause, RenamedItem};
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let origin = root.path().join("origin");
    let usb = root.path().join("usb");
    let project = origin.join("project");
    fs::create_dir_all(project.join("x:y"))?;
    fs::create_dir(&usb)?;
    for name in ["x:y/a:b", "CON", "taken", "plain", "long", "bytes"] {
        fs::write(project.join(name), name)?;
    }
    let _usb = FsQuirks::install(&usb, Quirks::vfat());
    let mut record = moved_onto_fat(&project, &usb);
    let moved = usb.join("project");
    let entry = |requested: OsString, saved_as: &str, cause| RenamedItem {
        requested: Location::local(moved.join(requested)),
        saved_as: Location::local(moved.join(saved_as)),
        cause,
    };
    let long = OsString::from("l".repeat(300));
    let bytes = OsString::from_vec(b"b\xffytes".to_vec());
    record.renamed.extend([
        entry("taken".into(), "plain", RenameCause::Invalid),
        entry(long.clone(), "long", RenameCause::TooLong),
        entry(bytes.clone(), "bytes", RenameCause::Invalid),
    ]);
    // As a share's location, which GIO renames.
    record.original = Location::uri(gio::File::for_path(&project).uri());

    let report = followed_report(&undone(record));

    assert_eq!(
        names(&project)?,
        ["CON", "bytes", "long", "plain", "taken", "x:y"]
    );
    assert_eq!(fs::read(project.join("x:y/a:b"))?, b"x:y/a:b");
    let mut renamed = report
        .renamed
        .iter()
        .map(|item| {
            (
                item.requested.file_name(),
                item.saved_as.file_name(),
                item.cause,
            )
        })
        .collect::<Vec<_>>();
    renamed.sort_by(|left, right| left.1.cmp(&right.1));
    assert_eq!(
        renamed,
        [
            (Some(bytes), Some("bytes".into()), RenameCause::Invalid),
            (Some(long), Some("long".into()), RenameCause::TooLong),
            (
                Some("taken".into()),
                Some("plain".into()),
                RenameCause::Taken
            ),
        ]
    );
    assert!(report.kept.is_empty(), "{report:?}");
    Ok(())
}
