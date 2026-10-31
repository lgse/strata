// SPDX-License-Identifier: MIT

use super::*;
use crate::services::ShownName;

#[test]
fn transfers_into_the_same_location_or_a_descendant_are_noops() {
    let source = gio::File::for_path("/fixture/source");
    let parent = gio::File::for_path("/fixture");
    let same_target = parent.child("source");
    let descendant = gio::File::for_path("/fixture/source/nested");
    let descendant_target = descendant.child("source");
    let unrelated = gio::File::for_path("/elsewhere");
    let unrelated_target = unrelated.child("source");

    assert!(transfer_is_noop(&source, &parent, &same_target));
    assert!(transfer_is_noop(&source, &source, &source.child("source")));
    assert!(transfer_is_noop(&source, &descendant, &descendant_target));
    assert!(!transfer_is_noop(&source, &unrelated, &unrelated_target));
}

#[test]
fn transfer_noops_resolve_aliased_destinations_but_not_symlink_sources()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let docs = root.path().join("docs");
    fs::create_dir_all(docs.join("Backup"))?;
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&docs, &link)?;
    let docs = gio::File::for_path(&docs);
    let link = gio::File::for_path(&link);
    let aliased_backup = link.child("Backup");

    assert!(transfer_is_noop(
        &docs,
        &aliased_backup,
        &aliased_backup.child("docs")
    ));
    assert!(!transfer_is_noop(&link, &docs, &docs.child("link")));
    Ok(())
}

#[test]
fn each_transfer_item_keeps_its_own_conflict_decision() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-conflict-decisions-test-{unique}"));
    let sources = root.join("sources");
    let destination = root.join("destination");
    fs::create_dir_all(&sources)?;
    fs::create_dir_all(&destination)?;
    fs::write(sources.join("replace.txt"), b"new replacement")?;
    fs::write(sources.join("late.txt"), b"new late item")?;
    fs::write(destination.join("replace.txt"), b"old replacement")?;
    fs::write(destination.join("late.txt"), b"late arrival")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(1),
            destination: Location::local(&destination),
            items: vec![
                PasteItem {
                    source: Location::local(sources.join("replace.txt")),
                    conflict: TransferConflict::ReplaceExisting,
                    target_name: None,
                },
                PasteItem {
                    source: Location::local(sources.join("late.txt")),
                    conflict: TransferConflict::FailIfExists,
                    target_name: None,
                },
            ],
            move_sources: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. }
                | OperationEvent::Cancelled { .. }
                | OperationEvent::TransferFailed { .. }
                | OperationEvent::Failed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    // Items without a conflict go first, so the late arrival stops the paste before Replace.
    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::TransferFailed {
            completed_locations,
            ..
        }) if completed_locations.is_empty()
    ));
    assert!(item_moves(&events.borrow()).is_empty());
    assert!(trashed_items(&events.borrow()).is_empty());
    assert_eq!(
        fs::read(destination.join("replace.txt"))?,
        b"old replacement"
    );
    assert_eq!(fs::read(destination.join("late.txt"))?, b"late arrival");
    assert!(sources.join("replace.txt").exists());
    assert!(sources.join("late.txt").exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn cutting_in_the_same_folder_remains_a_noop() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let file = destination.join("document.txt");
    let directory = destination.join("folder");
    fs::write(&file, b"content")?;
    fs::create_dir_all(&directory)?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(14),
            destination: Location::local(&destination),
            items: vec![
                PasteItem {
                    source: Location::local(&file),
                    conflict: TransferConflict::FailIfExists,
                    target_name: None,
                },
                PasteItem {
                    source: Location::local(&directory),
                    conflict: TransferConflict::FailIfExists,
                    target_name: None,
                },
            ],
            move_sources: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. } | OperationEvent::TransferFailed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(item_moves(&events.borrow()).is_empty());
    assert!(file.exists());
    assert!(directory.is_dir());
    assert!(!destination.join("document (1).txt").exists());
    assert!(!destination.join("folder (1)").exists());
    Ok(())
}

#[test]
fn keeping_both_preserves_transfer_noops() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let nested = source.join("nested");
    fs::create_dir_all(&nested)?;
    fs::write(source.join("report.txt"), b"original")?;

    for moving in [false, true] {
        for destination in [&source, &nested] {
            let created = run_paste_collecting_created(PasteRequest {
                id: OperationRequestId(77),
                destination: Location::local(destination),
                items: vec![PasteItem {
                    source: Location::local(&source),
                    conflict: TransferConflict::KeepBoth,
                    target_name: None,
                }],
                move_sources: moving,
            })?;
            assert!(created.into_iter().flatten().next().is_none());
            assert_eq!(fs::read_dir(&source)?.count(), 2);
            assert_eq!(fs::read_dir(&nested)?.count(), 0);
            assert_eq!(fs::read(source.join("report.txt"))?, b"original");
        }
    }
    Ok(())
}

#[test]
fn keeping_both_in_a_cross_folder_paste_generates_a_unique_name() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let source = source_dir.join("report.txt");
    let destination = root.path().join("dest");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&destination)?;
    fs::write(&source, b"incoming")?;
    fs::write(destination.join("report.txt"), b"existing")?;

    let created = run_paste_collecting_created(PasteRequest {
        id: OperationRequestId(74),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(&source),
            conflict: TransferConflict::KeepBoth,
            target_name: None,
        }],
        move_sources: false,
    })?;

    assert_eq!(
        created.into_iter().flatten().collect::<Vec<_>>(),
        vec![Location::local(destination.join("report (1).txt"))]
    );
    assert_eq!(fs::read(destination.join("report.txt"))?, b"existing");
    assert_eq!(fs::read(destination.join("report (1).txt"))?, b"incoming");
    assert!(source.exists());
    Ok(())
}

#[test]
fn keeping_both_while_moving_renames_the_destination_instead_of_replacing_it()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let source = source_dir.join("report.txt");
    let destination = root.path().join("dest");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&destination)?;
    fs::write(&source, b"incoming")?;
    fs::write(destination.join("report.txt"), b"existing")?;

    let events = run_paste(PasteRequest {
        id: OperationRequestId(75),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(&source),
            conflict: TransferConflict::KeepBoth,
            target_name: None,
        }],
        move_sources: true,
    });

    assert_eq!(
        item_moves(&events),
        [(
            Location::local(&source),
            Location::local(destination.join("report (1).txt"))
        )]
    );
    assert_eq!(fs::read(destination.join("report.txt"))?, b"existing");
    assert_eq!(fs::read(destination.join("report (1).txt"))?, b"incoming");
    assert!(!source.exists());
    Ok(())
}

#[test]
fn keeping_both_numbers_a_long_name_within_the_limit_of_its_mount() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let repeat = |text: &str, count| text.repeat(count);
    let rows = vec![
        (
            format!("{}.txt", repeat("a", 251)),
            false,
            format!("{} (1).txt", repeat("a", 247)),
        ),
        (repeat("d", 254), true, format!("{} (1)", repeat("d", 251))),
        (
            format!("{}.tar.gz", repeat("a", 247)),
            false,
            format!("{}.tar (1).gz", repeat("a", 244)),
        ),
    ];
    let samba_folder = vec![(repeat("d", 212), true, format!("{} (1)", repeat("d", 209)))];
    let ntfs_3g = || Quirks {
        rename_flags: fs_quirks::Support::Rejected(rustix::io::Errno::INVAL),
        ..Quirks::ntfs_3g()
    };
    let mounts: [(&str, Option<Preset>, _); 4] = [
        ("host", None, &rows),
        ("vfat", Some(Quirks::vfat), &rows),
        ("cifs", Some(Quirks::cifs), &samba_folder),
        ("ntfs-3g", Some(ntfs_3g), &rows),
    ];
    for (mount, preset, rows) in mounts {
        for moving in [false, true] {
            for (name, folder, numbered) in rows {
                let context = format!("{mount}, moving: {moving}, {name}");
                let root = tempfile::tempdir()?;
                let _quirks = preset.map(|preset| FsQuirks::install(root.path(), preset()));
                let sources = root.path().join("sources");
                let source = sources.join(name);
                let destination = root.path().join("destination");
                fs::create_dir(&sources)?;
                fs::create_dir(&destination)?;
                let incoming = if *folder {
                    fs::create_dir(&source)?;
                    fs::create_dir(destination.join(name))?;
                    fs::write(source.join("inside.txt"), b"incoming")?;
                    destination.join(numbered).join("inside.txt")
                } else {
                    fs::write(&source, b"incoming")?;
                    fs::write(destination.join(name), b"existing")?;
                    destination.join(numbered)
                };

                run_paste(PasteRequest {
                    id: OperationRequestId(78),
                    destination: Location::local(&destination),
                    items: vec![PasteItem {
                        source: Location::local(&source),
                        conflict: TransferConflict::KeepBoth,
                        target_name: None,
                    }],
                    move_sources: moving,
                });

                let mut expected = vec![name.clone(), numbered.clone()];
                expected.sort();
                assert_eq!(names(&destination)?, expected, "{context}");
                assert_eq!(fs::read(incoming)?, b"incoming", "{context}");
                assert_eq!(source.exists(), !moving, "{context}");
            }
        }
    }
    Ok(())
}

#[test]
fn mixed_conflict_choices_apply_independently_across_a_multi_item_paste()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir_all(&sources)?;
    fs::create_dir_all(&destination)?;
    fs::write(sources.join("new.txt"), b"brand new")?;
    fs::write(sources.join("replace.txt"), b"new replacement")?;
    fs::write(destination.join("replace.txt"), b"old replacement")?;
    fs::write(sources.join("keep.txt"), b"new keep")?;
    fs::write(destination.join("keep.txt"), b"old keep")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(76),
            destination: Location::local(&destination),
            items: vec![
                PasteItem {
                    source: Location::local(sources.join("new.txt")),
                    conflict: TransferConflict::FailIfExists,
                    target_name: None,
                },
                PasteItem {
                    source: Location::local(sources.join("replace.txt")),
                    conflict: TransferConflict::ReplaceExisting,
                    target_name: None,
                },
                PasteItem {
                    source: Location::local(sources.join("keep.txt")),
                    conflict: TransferConflict::KeepBoth,
                    target_name: None,
                },
            ],
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. }
                | OperationEvent::Cancelled { .. }
                | OperationEvent::TransferFailed { .. }
                | OperationEvent::Failed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert_eq!(fs::read(destination.join("new.txt"))?, b"brand new");
    assert_eq!(
        fs::read(destination.join("replace.txt"))?,
        b"new replacement"
    );
    assert_eq!(fs::read(destination.join("keep.txt"))?, b"old keep");
    assert_eq!(fs::read(destination.join("keep (1).txt"))?, b"new keep");
    Ok(())
}

#[test]
fn pasting_onto_itself_with_replace_is_a_noop() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let source = destination.join("todo.txt");
    fs::write(&source, b"existing\n")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(32),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::ReplaceExisting,
                target_name: None,
            }],
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. } | OperationEvent::TransferFailed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(source.exists());
    assert_eq!(fs::read(&source)?, b"existing\n");
    assert!(!destination.join("todo (1).txt").exists());
    Ok(())
}

#[test]
fn pasting_onto_itself_with_keep_both_creates_numbered_copy() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let source = destination.join("todo.txt");
    fs::write(&source, b"existing\n")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(33),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::KeepBoth,
                target_name: None,
            }],
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. } | OperationEvent::TransferFailed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(source.exists());
    assert_eq!(fs::read(&source)?, b"existing\n");
    assert_eq!(fs::read(destination.join("todo (1).txt"))?, b"existing\n");
    Ok(())
}

fn taking(path: &Path) -> fs_quirks::Racer {
    let path = path.to_path_buf();
    Box::new(move || {
        if fs::symlink_metadata(&path).is_err() {
            fs::write(&path, b"taken meanwhile").expect("take the name");
        }
    })
}

#[test]
fn keep_both_copy_renumbers_when_its_name_is_taken_meanwhile() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unnamed_files_refused: Preset = || Quirks {
        tmpfile: fs_quirks::Support::Rejected(rustix::io::Errno::OPNOTSUPP),
        ..Quirks::ext4()
    };
    let rows: [(&str, &str, Preset); 3] = [
        ("report.txt", "report (1).txt", Quirks::default),
        ("report.txt", "report (1).txt", unnamed_files_refused),
        ("docs", "docs (1)", Quirks::default),
    ];
    for (name, first, quirks) in rows {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir_all(&sources)?;
        fs::create_dir_all(&destination)?;
        let source = sources.join(name);
        if name == "docs" {
            fs::create_dir(&source)?;
            fs::write(source.join("inside.txt"), b"incoming")?;
            fs::create_dir(destination.join(name))?;
        } else {
            fs::write(&source, b"incoming")?;
            fs::write(destination.join(name), b"existing")?;
        }
        let taken = destination.join(first);
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                before: vec![
                    (fs_quirks::Op::LinkFd, first, 0, taking(&taken)),
                    (fs_quirks::Op::Rename, first, 0, taking(&taken)),
                ],
                ..quirks()
            },
        );

        let created = run_paste_collecting_created(PasteRequest {
            id: OperationRequestId(77),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::KeepBoth,
                target_name: None,
            }],
            move_sources: false,
        })?;

        let second = first.replace("(1)", "(2)");
        assert_eq!(
            created.into_iter().flatten().collect::<Vec<_>>(),
            [Location::local(destination.join(&second))],
            "{name}"
        );
        assert_eq!(fs::read(&taken)?, b"taken meanwhile", "{name}");
        let copied = destination.join(&second);
        let copied = if name == "docs" {
            copied.join("inside.txt")
        } else {
            copied
        };
        assert_eq!(fs::read(copied)?, b"incoming", "{name}");
    }
    Ok(())
}

#[test]
fn keep_both_takes_the_next_number_past_names_already_taken() -> Result<(), Box<dyn Error>> {
    use crate::adapters::volume::learn_mount_capability;
    use fs_quirks::Op;
    use std::os::unix::fs::FileTypeExt;
    let _serial = serial()?;
    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        File,
        Folder,
        Link,
        Pipe,
    }
    /// A move renames within one volume, so its folder is numbered as one;
    /// copies cross volumes, and a folder is made in place where the mount
    /// is known to place none.
    #[derive(Clone, Copy, PartialEq)]
    enum Way {
        Moved,
        Copied,
        InPlace,
    }
    // Where a stage cannot be placed, it is copied out to a name another
    // program takes first.
    type Row = (
        &'static str,
        Kind,
        Way,
        Preset,
        &'static [&'static str],
        Option<(Op, &'static str, usize)>,
        &'static str,
    );
    let rows: [Row; 7] = [
        (
            "my.photos",
            Kind::Folder,
            Way::Moved,
            Quirks::default,
            &["my.photos"],
            None,
            "my.photos (1)",
        ),
        (
            "x (2).txt",
            Kind::File,
            Way::Copied,
            Quirks::default,
            &["x (2).txt"],
            None,
            "x (3).txt",
        ),
        (
            "link",
            Kind::Link,
            Way::Copied,
            Quirks::default,
            &["link", "link (1)"],
            None,
            "link (2)",
        ),
        (
            "pipe",
            Kind::Pipe,
            Way::Copied,
            Quirks::default,
            &["pipe", "pipe (1)"],
            None,
            "pipe (2)",
        ),
        (
            "docs",
            Kind::Folder,
            Way::InPlace,
            Quirks::without_folder_placement,
            &["docs", "docs (1)"],
            None,
            "docs (2)",
        ),
        (
            "docs",
            Kind::Folder,
            Way::Copied,
            Quirks::without_placement,
            &["docs"],
            Some((Op::Mkdir, "docs (1)", 1)),
            "docs (2)",
        ),
        (
            "a.txt",
            Kind::File,
            Way::Copied,
            Quirks::without_placement,
            &["a.txt"],
            Some((Op::Create, "a (1).txt", 0)),
            "a (2).txt",
        ),
    ];
    for (name, kind, way, preset, taken, racer, expected) in rows {
        let row = format!("{name} -> {expected}");
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir_all(&sources)?;
        fs::create_dir_all(&destination)?;
        let source = sources.join(name);
        match kind {
            Kind::File => fs::write(&source, b"incoming")?,
            Kind::Folder => {
                fs::create_dir(&source)?;
                fs::write(source.join("inside.txt"), b"incoming")?;
            }
            Kind::Link => std::os::unix::fs::symlink("incoming", &source)?,
            Kind::Pipe => rustix::fs::mknodat(
                rustix::fs::CWD,
                &source,
                rustix::fs::FileType::Fifo,
                rustix::fs::Mode::from_raw_mode(0o600),
                0,
            )?,
        }
        let mut existing = taken.to_vec();
        for name in taken {
            fs::write(destination.join(name), b"existing")?;
        }
        let mut quirks = preset();
        if let Some((op, first, skip)) = racer {
            existing.push(first);
            quirks.before = vec![(op, first, skip, taking(&destination.join(first)))];
        }
        let moving = way == Way::Moved;
        let _sources = (!moving).then(|| FsQuirks::install(&sources, Quirks::default()));
        let scope = FsQuirks::install(if moving { root.path() } else { &destination }, quirks);
        if way == Way::InPlace {
            learn_mount_capability(scope.key(), |capabilities| {
                capabilities.no_replace = Some(false);
                capabilities.hard_links = Some(true);
                capabilities.rename_over_empty_directory = Some(false);
            });
        }

        run_paste(PasteRequest {
            id: OperationRequestId(79),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::KeepBoth,
                target_name: None,
            }],
            move_sources: moving,
        });

        existing.push(expected);
        existing.sort_unstable();
        assert_eq!(names(&destination)?, existing, "{row}");
        let copied = destination.join(expected);
        match kind {
            Kind::File => assert_eq!(fs::read(&copied)?, b"incoming", "{row}"),
            Kind::Folder => assert_eq!(fs::read(copied.join("inside.txt"))?, b"incoming", "{row}"),
            Kind::Link => assert_eq!(fs::read_link(&copied)?, Path::new("incoming"), "{row}"),
            Kind::Pipe => assert!(
                fs::symlink_metadata(&copied)?.file_type().is_fifo(),
                "{row}"
            ),
        }
    }
    Ok(())
}

#[test]
fn replace_and_merge_act_on_the_name_the_prompt_confirmed() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let (replace, merge) = (TransferConflict::ReplaceExisting, TransferConflict::Merge);
    let (vfat, ntfs3, cifs): (Preset, Preset, Preset) = (Quirks::vfat, Quirks::ntfs3, Quirks::cifs);
    for (preset, quirks, confirmed, fixed, conflict) in [
        ("vfat", vfat, "CON", Some("CON_"), replace),
        ("ntfs3", ntfs3, "a:b", Some("a_b"), replace),
        ("ntfs3", ntfs3, "a:b", None, replace),
        ("cifs", cifs, "a:b", Some("a_b"), replace),
        ("ntfs3", ntfs3, "x:y", Some("x_y"), merge),
        ("ntfs3", ntfs3, "x:y", None, merge),
    ] {
        let context = format!("{preset} {confirmed} beside {fixed:?}, {conflict:?}");
        let root = tempfile::tempdir()?;
        let (source, disk) = (root.path().join("source"), root.path().join("disk"));
        fs::create_dir_all(&source)?;
        fs::create_dir_all(&disk)?;
        let merging = conflict == TransferConflict::Merge;
        let file = |item: &str| {
            if merging {
                format!("{item}/same.txt")
            } else {
                item.to_owned()
            }
        };
        for item in std::iter::once(confirmed).chain(fixed) {
            if merging {
                fs::create_dir_all(disk.join(item))?;
            }
            fs::write(disk.join(file(item)), item)?;
        }
        if merging {
            fs::create_dir(source.join(confirmed))?;
        }
        fs::write(source.join(file(confirmed)), "incoming")?;
        let _disk = FsQuirks::install(&disk, quirks());

        run_paste(PasteRequest {
            id: OperationRequestId(1615),
            destination: Location::local(&disk),
            items: vec![PasteItem {
                source: Location::local(source.join(confirmed)),
                conflict,
                target_name: None,
            }],
            move_sources: false,
        });

        assert_eq!(
            fs::read(disk.join(file(confirmed)))?,
            b"incoming",
            "{context}"
        );
        if let Some(fixed) = fixed {
            assert_eq!(
                fs::read(disk.join(file(fixed)))?,
                fixed.as_bytes(),
                "{context}"
            );
        }
        assert_eq!(
            names(&disk)?.len(),
            1 + usize::from(fixed.is_some()),
            "{context}"
        );
    }
    Ok(())
}

#[test]
fn the_prompt_sees_the_name_a_paste_gives_each_item_and_what_has_it() -> Result<(), Box<dyn Error>>
{
    use super::super::transfer_targets::{
        Collision, LOOKUPS_WITHOUT_LISTING, TransferTarget, transfer_targets,
    };
    use crate::services::RenameCause;
    let _serial = serial()?;
    let file = Some(Collision { mergeable: false });
    let folder = Some(Collision { mergeable: true });
    let (vfat, ext4): (Preset, Preset) = (Quirks::vfat, Quirks::ext4);
    // Per source: its name and what it meets on vfat, then on ext4, and
    // whether names inside it change on vfat. `own.txt` and `same` are pasted
    // from the destination itself and `old.txt` from a Trash on another disk.
    let rows = [
        ("a:b", ("a_b", file), ("a:b", None), false),
        ("Photo.jpg", ("Photo.jpg", file), ("Photo.jpg", None), false),
        ("x:y", ("x_y", folder), ("x:y", None), true),
        ("folder", ("folder", folder), ("folder", folder), true),
        ("same", ("same", folder), ("same", folder), false),
        ("data", ("data", file), ("data", file), false),
        ("link", ("link", None), ("link", file), false),
        ("new.txt", ("new.txt", None), ("new.txt", None), false),
        ("own.txt", ("own.txt", None), ("own.txt", None), false),
        ("old.txt", ("old.txt", file), ("old.txt", file), false),
    ];
    // Enough more items make the lookup read the destination's listing.
    for ((preset, on_vfat), padding) in [(vfat, true), (ext4, false)]
        .into_iter()
        .flat_map(|mount| [(mount, 0), (mount, LOOKUPS_WITHOUT_LISTING)])
    {
        let pads = (0..padding)
            .map(|index| format!("pad-{index:02}"))
            .collect::<Vec<_>>();
        let root = tempfile::tempdir()?;
        let (source, disk) = (root.path().join("source"), root.path().join("disk"));
        for name in ["x:y", "folder"] {
            fs::create_dir_all(source.join(name))?;
        }
        for name in ["a:b", "Photo.jpg", "data", "new.txt"]
            .into_iter()
            .chain(pads.iter().map(String::as_str))
        {
            fs::write(source.join(name), name)?;
        }
        std::os::unix::fs::symlink("a:b", source.join("link"))?;
        for name in ["x_y", "folder", "data", "same", "inner/same"] {
            fs::create_dir_all(disk.join(name))?;
        }
        for name in ["a_b", "photo.JPG", "link", "own.txt", "old.txt"] {
            fs::write(disk.join(name), name)?;
        }
        let disk_quirks = FsQuirks::install(&disk, preset());
        let sources = rows
            .iter()
            .map(|(name, ..)| match *name {
                "own.txt" => Location::local(disk.join(name)),
                "same" => Location::local(disk.join("inner").join(name)),
                "old.txt" => Location::uri(format!(
                    "trash:///%5Cmnt%5CData%5C.Trash-1000%5Cfiles%5C{name}"
                )),
                _ => Location::local(source.join(name)),
            })
            .chain(pads.iter().map(|pad| Location::local(source.join(pad))))
            .collect::<Vec<_>>();

        let targets = glib::MainContext::default().block_on(transfer_targets(
            &Location::local(&disk),
            &sources,
            false,
            &gio::Cancellable::new(),
        ))?;

        let expected = rows
            .iter()
            .map(|(item, vfat, ext4, fixes_inside)| {
                let (name, collision) = if on_vfat { *vfat } else { *ext4 };
                TransferTarget {
                    name: Some(ShownName {
                        name: OsString::from(name),
                        cause: (name != *item).then_some(RenameCause::Invalid),
                    }),
                    collision,
                    fixes_inside: on_vfat && *fixes_inside,
                }
            })
            .chain(pads.iter().map(|pad| TransferTarget {
                name: Some(ShownName {
                    name: OsString::from(pad),
                    cause: None,
                }),
                collision: None,
                fixes_inside: false,
            }))
            .collect::<Vec<_>>();
        assert_eq!(targets, expected, "vfat {on_vfat}, {padding} more items");
        assert_eq!(
            disk_quirks.count(super::fs_quirks::Op::Names),
            usize::from(padding > 0),
            "vfat {on_vfat}, {padding} more items"
        );
    }
    Ok(())
}

#[test]
fn a_paste_gives_an_item_the_name_its_prompt_showed() -> Result<(), Box<dyn Error>> {
    use crate::services::RenameCause::{Alias, Invalid};
    let _serial = serial()?;
    let (replace, keep_both, merge, fail) = (
        TransferConflict::ReplaceExisting,
        TransferConflict::KeepBoth,
        TransferConflict::Merge,
        TransferConflict::FailIfExists,
    );
    // The item, how it collided, the name the prompt showed, whether it is
    // moved, where it lands and why.
    for (item, conflict, shown, moving, landed, cause) in [
        ("a:b", replace, "a_b", false, "a_b", Invalid),
        ("a:b", keep_both, "a_b", false, "a_b (1)", Invalid),
        ("a:b", keep_both, "a_b", true, "a_b (1)", Invalid),
        ("a:b", fail, "a_b (1)", false, "a_b (1)", Invalid),
        ("b.jpg", fail, "b (1).jpg", false, "b (1).jpg", Alias),
        ("x:y", merge, "x_y", false, "x_y", Invalid),
    ] {
        let context = format!("{item} as {shown}, {conflict:?}, moving {moving}");
        let root = tempfile::tempdir()?;
        let (source, disk) = (root.path().join("source"), root.path().join("disk"));
        fs::create_dir_all(&source)?;
        fs::create_dir_all(&disk)?;
        let merging = conflict == merge;
        let file = |item: &str| {
            if merging {
                format!("{item}/same.txt")
            } else {
                item.to_owned()
            }
        };
        if merging {
            fs::create_dir(source.join(item))?;
            fs::create_dir(disk.join(shown))?;
        }
        fs::write(source.join(file(item)), "incoming")?;
        fs::write(disk.join(file(&shown.replace(" (1)", ""))), "old")?;
        let _disk = FsQuirks::install(&disk, Quirks::vfat());

        let events = run_paste(PasteRequest {
            id: OperationRequestId(1615),
            destination: Location::local(&disk),
            items: vec![PasteItem {
                source: Location::local(source.join(item)),
                conflict,
                target_name: Some(ShownName {
                    name: OsString::from(shown),
                    cause: None,
                }),
            }],
            move_sources: moving,
        });

        assert_eq!(fs::read(disk.join(file(landed)))?, b"incoming", "{context}");
        assert_eq!(
            names(&disk)?.len(),
            1 + usize::from(landed != shown || conflict == fail),
            "{context}"
        );
        assert_eq!(source.join(item).exists(), !moving, "{context}");
        assert_eq!(
            renamed_beneath(&disk, &followed_report(&events)),
            vec![(item.to_owned(), landed.to_owned(), cause)],
            "{context}"
        );
    }
    Ok(())
}

#[test]
fn a_paste_fails_an_item_whose_shown_name_is_not_one_plain_name() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for shown in ["../escaped", "sub/x", "a\0b"] {
        let root = tempfile::tempdir()?;
        let (source, disk) = (root.path().join("source"), root.path().join("disk"));
        fs::create_dir_all(&source)?;
        fs::create_dir_all(disk.join("sub"))?;
        fs::write(source.join("item"), "incoming")?;

        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let _operation = LocalOperationProvider.paste(
            PasteRequest {
                id: OperationRequestId(1615),
                destination: Location::local(&disk),
                items: vec![PasteItem {
                    source: Location::local(source.join("item")),
                    conflict: TransferConflict::FailIfExists,
                    target_name: Some(ShownName {
                        name: OsString::from(shown),
                        cause: None,
                    }),
                }],
                move_sources: false,
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );
        wait_for_operation(&events, |event| {
            matches!(
                event,
                OperationEvent::Pasted { .. }
                    | OperationEvent::TransferFailed { .. }
                    | OperationEvent::Failed { .. }
            )
        });

        assert!(
            matches!(events.borrow().last(), Some(OperationEvent::Failed { .. })),
            "{shown:?}"
        );
        assert_eq!(names(root.path())?, ["disk", "source"], "{shown:?}");
        assert_eq!(names(&disk)?, ["sub"], "{shown:?}");
        assert!(names(&disk.join("sub"))?.is_empty(), "{shown:?}");
    }
    Ok(())
}
