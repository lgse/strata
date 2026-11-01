// SPDX-License-Identifier: MIT

use std::{
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    sync::atomic::Ordering,
};

use rustix::io::Errno;

use super::fs_quirks::{NameOpens, Op, Support, hold};
use super::*;
use crate::adapters::volume::{Family, mount_capabilities};
use crate::services::{Holder, KeptCause, KeptItem, SpecialKind, TransferReport};

#[test]
fn moving_a_directory_falls_back_to_a_safe_copy_when_the_move_needs_one()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-move-fallback-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::write(source.join("top.txt"), b"top")?;
    fs::write(source.join("nested/child.txt"), b"child")?;

    let result = glib::MainContext::default().block_on(move_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        always_copy(),
    ));

    assert!(result.is_ok());
    assert!(!source.exists());
    assert_eq!(fs::read(target.join("top.txt"))?, b"top");
    assert_eq!(fs::read(target.join("nested/child.txt"))?, b"child");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_failed_move_attempt_is_returned_without_falling_back() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-move-real-failure-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&source)?;
    fs::write(source.join("top.txt"), b"top")?;

    let result = glib::MainContext::default().block_on(move_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        Rc::new(|_, _, _, _| {
            Box::pin(async {
                Err(glib::Error::new(
                    gio::IOErrorEnum::PermissionDenied,
                    "injected permission failure",
                ))
            })
        }),
    ));

    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::PermissionDenied)));
    assert_eq!(fs::read(source.join("top.txt"))?, b"top");
    assert!(!target.exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_successful_move_attempt_is_used_without_falling_back_to_copy() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-move-success-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&source)?;
    fs::write(source.join("top.txt"), b"top")?;

    let result = glib::MainContext::default().block_on(move_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        Rc::new(|source, target, _, _| {
            Box::pin(async move {
                fs::rename(
                    source.path().expect("native source"),
                    target.path().expect("native target"),
                )
                .map(|()| super::super::Moved::Done(target))
                .map_err(super::io_error)
            })
        }),
    ));

    assert!(result.is_ok());
    assert!(!source.exists());
    assert_eq!(fs::read(target.join("top.txt"))?, b"top");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_plain_move_relocates_the_entry_via_the_hardened_rename_path() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    let target = root.path().join("target.txt");
    fs::write(&source, b"payload")?;

    let result = glib::MainContext::default().block_on(move_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        Taken::Fail,
        gio::Cancellable::new(),
        None,
        NamePolicy::default(),
    ));

    assert!(result.is_ok());
    assert!(!source.exists());
    assert_eq!(fs::read(target)?, b"payload");
    Ok(())
}

fn move_native(source: &Path, target: &Path) -> Result<(), glib::Error> {
    glib::MainContext::default()
        .block_on(move_local(
            gio::File::for_path(source),
            gio::File::for_path(target),
            Taken::Fail,
            gio::Cancellable::new(),
            None,
            NamePolicy::default(),
        ))
        .map(drop)
}

fn assert_refused_into_itself(result: Result<(), glib::Error>) {
    assert_eq!(
        result.map_err(|error| error.message().to_owned()),
        Err(crate::i18n::tr("Can\u{2019}t put a folder inside itself"))
    );
}

#[test]
fn moving_a_directory_into_its_own_child_fails_instead_of_deleting_it() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    for quirks in [Quirks::default(), Quirks::vboxsf()] {
        let root = tempfile::tempdir()?;
        let _scope = FsQuirks::install(root.path(), quirks);
        let source = root.path().join("source");
        fs::create_dir_all(source.join("nested"))?;
        fs::write(source.join("top.txt"), b"top")?;
        let target = source.join("nested").join("moved-source");

        assert_refused_into_itself(move_native(&source, &target));
        assert_eq!(fs::read(source.join("top.txt"))?, b"top");
        assert!(!target.exists());
        assert_eq!(fs::read_dir(source.join("nested"))?.count(), 0);
    }
    Ok(())
}

#[test]
fn a_move_never_copies_a_folder_into_itself_across_a_nested_mount() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let docs = root.path().join("docs");
    fs::create_dir_all(docs.join("Backup"))?;
    fs::write(docs.join("top.txt"), b"top")?;
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&docs, &link)?;
    let _outer = FsQuirks::install(root.path(), Quirks::default());
    let _nested = FsQuirks::install(&docs.join("Backup"), Quirks::default());

    for target in [docs.join("Backup/docs"), link.join("Backup/docs")] {
        assert_refused_into_itself(move_native(&docs, &target));
        assert_eq!(fs::read(docs.join("top.txt"))?, b"top");
        assert_eq!(fs::read_dir(docs.join("Backup"))?.count(), 0);
    }
    Ok(())
}

#[test]
fn same_volume_move_on_a_flag_rejecting_filesystem_never_copies() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for rejection in [Errno::INVAL, Errno::NOSYS, Errno::OPNOTSUPP] {
        let root = tempfile::tempdir()?;
        let scope = FsQuirks::install(
            root.path(),
            Quirks {
                rename_flags: Support::Rejected(rejection),
                ..Quirks::default()
            },
        );
        let file = root.path().join("notes.txt");
        let folder = root.path().join("folder");
        fs::write(&file, b"notes")?;
        fs::create_dir(&folder)?;
        fs::write(folder.join("inside.txt"), b"inside")?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let file_inode = fs::metadata(&file)?.ino();
        let folder_inode = fs::metadata(&folder)?.ino();

        move_native(&file, &destination.join("notes.txt"))?;
        move_native(&folder, &destination.join("folder"))?;

        assert_eq!(
            fs::metadata(destination.join("notes.txt"))?.ino(),
            file_inode
        );
        assert_eq!(
            fs::metadata(destination.join("folder"))?.ino(),
            folder_inode
        );
        assert_eq!(fs::read(destination.join("folder/inside.txt"))?, b"inside");
        assert!(!file.exists() && !folder.exists());
        assert_eq!(
            scope.count(Op::Create),
            usize::from(rejection == Errno::INVAL)
        );
    }
    Ok(())
}

#[test]
fn items_without_noreplace_or_a_usable_link_are_renamed_without_copying()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // As protected_hardlinks refuses another user's file that a rename could move.
    let refused = || Quirks {
        fail: vec![(Op::Link, "moved", 0, Errno::PERM)],
        ..Quirks::without_rename_flags()
    };
    let linked_before_a_probe = || Quirks {
        rename_flags: Support::Rejected(Errno::NOSYS),
        ..Quirks::sshfs()
    };
    let ntfs_3g = || Quirks {
        rename_flags: Support::Rejected(Errno::INVAL),
        ..Quirks::ntfs_3g()
    };
    let cut = "m".repeat(237);
    let cases: [(&str, Preset, bool, &str); 7] = [
        ("refused", refused, false, "moved"),
        ("sshfs", Quirks::sshfs, false, "moved"),
        ("sshfs", Quirks::sshfs, true, "moved"),
        (
            "linked before a probe",
            linked_before_a_probe,
            false,
            "moved",
        ),
        ("vboxsf", Quirks::vboxsf, false, "moved"),
        ("vboxsf", Quirks::vboxsf, true, "moved"),
        ("ntfs-3g, a name it would cut", ntfs_3g, true, &cut),
    ];
    for (case, quirks, folder, name) in cases {
        let root = tempfile::tempdir()?;
        let _scope = FsQuirks::install(root.path(), quirks());
        let source = root.path().join("item");
        let moved = root.path().join(name);
        if folder {
            fs::create_dir(&source)?;
            fs::write(source.join("inside.txt"), b"inside")?;
        } else {
            fs::write(&source, b"notes")?;
        }
        let inode = fs::metadata(&source)?.ino();
        let progress =
            TransferProgressTracker::new(OperationRequestId(1), None, None, Rc::new(|_| {}));

        glib::MainContext::default()
            .block_on(move_local(
                gio::File::for_path(&source),
                gio::File::for_path(&moved),
                Taken::Fail,
                gio::Cancellable::new(),
                Some(progress.clone()),
                NamePolicy::default(),
            ))
            .map_err(|error| format!("{case}: {error}"))?;

        let case = format!("{case}, folder: {folder}");
        assert_eq!(fs::metadata(&moved)?.ino(), inode, "{case}");
        assert_eq!(progress.completed_files.get(), 0, "{case}");
        assert_eq!(progress.transferred_bytes.get(), 0, "{case}");
        assert_eq!(names(root.path())?, [name], "{case}");
    }
    Ok(())
}

#[test]
fn items_no_step_can_place_are_moved_by_copying() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // As SFTP without `posix-rename` refuses to rename over the reserved file.
    let rename_over_refused = || Quirks {
        fail: vec![(Op::Rename, "moved", 1, Errno::PERM)],
        ..Quirks::sshfs()
    };
    let cases: [(&str, Preset, bool); 3] = [
        ("links unsettled", Quirks::without_placement, false),
        ("rename over refused", rename_over_refused, false),
        (
            "folders unplaceable",
            Quirks::without_folder_placement,
            true,
        ),
    ];
    for (case, quirks, folder) in cases {
        let root = tempfile::tempdir()?;
        let _scope = FsQuirks::install(root.path(), quirks());
        let source = root.path().join("item");
        let moved = root.path().join("moved");
        if folder {
            fs::create_dir(&source)?;
            fs::write(source.join("inside.txt"), b"inside")?;
        } else {
            fs::write(&source, b"inside")?;
        }
        let inode = fs::metadata(&source)?.ino();

        move_native(&source, &moved).map_err(|error| format!("{case}: {error}"))?;

        assert_ne!(fs::metadata(&moved)?.ino(), inode, "{case}");
        let contents = if folder {
            moved.join("inside.txt")
        } else {
            moved.clone()
        };
        assert_eq!(fs::read(contents)?, b"inside", "{case}");
        assert_eq!(names(root.path())?, ["moved"], "{case}");
    }
    Ok(())
}

#[test]
fn einval_from_every_rung_reports_an_invalid_name_and_keeps_the_source()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let flags_supported = || Quirks {
        invalid_name: Some(has_colon),
        ..Quirks::default()
    };
    let without_flags_or_links = || Quirks {
        invalid_name: Some(has_colon),
        rename_flags: Support::Rejected(Errno::INVAL),
        links: Support::Rejected(Errno::PERM),
        ..Quirks::default()
    };
    for (quirks, kind) in [
        (flags_supported(), "file"),
        (flags_supported(), "folder"),
        (without_flags_or_links(), "folder"),
        (without_flags_or_links(), "symlink"),
    ] {
        let root = tempfile::tempdir()?;
        let _scope = FsQuirks::install(root.path(), quirks);
        let source = root.path().join("source");
        match kind {
            "file" => fs::write(&source, b"kept")?,
            "folder" => fs::create_dir(&source)?,
            _ => std::os::unix::fs::symlink("elsewhere", &source)?,
        }
        let target = root.path().join("a:b");

        let error = move_native(&source, &target).expect_err("invalid name");

        assert_eq!(
            error.message(),
            rust_i18n::t!(
                "Could not move %{name}: %{error}",
                name = "source",
                error = crate::services::error_detail(crate::i18n::tr(
                    crate::services::INVALID_NAME_REASON
                ))
            ),
            "{kind}"
        );
        assert!(fs::symlink_metadata(&source).is_ok(), "{kind}");
        assert!(fs::symlink_metadata(&target).is_err(), "{kind}");
        assert_eq!(fs::read_dir(root.path())?.count(), 1, "{kind}");
    }
    Ok(())
}

#[test]
fn move_accepts_a_symlink_in_the_sources_parent_path() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let actual_parent = root.path().join("actual");
    let linked_parent = root.path().join("linked");
    fs::create_dir(&actual_parent)?;
    fs::write(actual_parent.join("source.txt"), b"keep")?;
    std::os::unix::fs::symlink(&actual_parent, &linked_parent)?;
    let target = root.path().join("target.txt");

    let result = glib::MainContext::default().block_on(move_local(
        gio::File::for_path(linked_parent.join("source.txt")),
        gio::File::for_path(&target),
        Taken::Fail,
        gio::Cancellable::new(),
        None,
        NamePolicy::default(),
    ));

    assert!(result.is_ok(), "{result:?}");
    assert!(!actual_parent.join("source.txt").exists());
    assert_eq!(fs::read(target)?, b"keep");
    Ok(())
}

#[test]
fn move_accepts_a_symlink_in_the_destinations_parent_path() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    let actual_destination = root.path().join("actual");
    let linked_destination = root.path().join("linked");
    fs::create_dir(&actual_destination)?;
    fs::write(&source, b"keep")?;
    std::os::unix::fs::symlink(&actual_destination, &linked_destination)?;

    let result = glib::MainContext::default().block_on(move_local(
        gio::File::for_path(&source),
        gio::File::for_path(linked_destination.join("target.txt")),
        Taken::Fail,
        gio::Cancellable::new(),
        None,
        NamePolicy::default(),
    ));

    assert!(result.is_ok(), "{result:?}");
    assert!(!source.exists());
    assert_eq!(fs::read(actual_destination.join("target.txt"))?, b"keep");
    Ok(())
}

fn paste_move(
    id: u64,
    destination: &Path,
    source: &Path,
    conflict: TransferConflict,
) -> (LoadHandle, Rc<RefCell<Vec<OperationEvent>>>) {
    paste_moves(id, destination, &[source], conflict)
}

fn paste_moves(
    id: u64,
    destination: &Path,
    sources: &[&Path],
    conflict: TransferConflict,
) -> (LoadHandle, Rc<RefCell<Vec<OperationEvent>>>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(id),
            destination: Location::local(destination),
            items: sources
                .iter()
                .map(|source| PasteItem {
                    source: Location::local(source),
                    conflict,
                    target_name: None,
                })
                .collect(),
            move_sources: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    (operation, events)
}

#[test]
fn moves_delete_their_sources_only_once_the_destination_is_flushed() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let conflicts = [
        TransferConflict::FailIfExists,
        TransferConflict::ReplaceExisting,
        TransferConflict::Merge,
    ];
    for (id, conflict) in (90..).zip(conflicts) {
        for flushed in [true, false] {
            let row = format!("{conflict:?}, flushed: {flushed}");
            let root = tempfile::tempdir()?;
            let source_dir = root.path().join("source");
            let destination = root.path().join("usb");
            let (item, incoming) = if conflict == TransferConflict::Merge {
                (Path::new("folder"), Path::new("folder/incoming.txt"))
            } else {
                (Path::new("notes.txt"), Path::new("notes.txt"))
            };
            fs::create_dir_all(source_dir.join(incoming).parent().ok_or("a parent")?)?;
            fs::create_dir_all(&destination)?;
            fs::write(source_dir.join(incoming), b"incoming")?;
            match conflict {
                TransferConflict::ReplaceExisting => fs::write(destination.join(item), b"old")?,
                TransferConflict::Merge => {
                    fs::create_dir(destination.join(item))?;
                    fs::write(destination.join("folder/stays.txt"), b"keep")?;
                }
                _ => {}
            }
            let source = source_dir.join(item);
            let during = Arc::new(std::sync::Mutex::new(Vec::new()));
            let (record, watched) = (during.clone(), source.clone());
            let racer: super::fs_quirks::Racer = Box::new(move || {
                record
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(watched.exists());
            });
            let _source = FsQuirks::install(&source_dir, Quirks::default());
            let _destination = FsQuirks::install(
                &destination,
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

            let (_operation, events) = paste_move(id, &destination, &source, conflict);
            pump_until_transfer(&events);

            assert_eq!(
                *during.lock().unwrap_or_else(|error| error.into_inner()),
                [true],
                "{row}: one flush, before the source is deleted"
            );
            assert_eq!(fs::read(destination.join(incoming))?, b"incoming", "{row}");
            if conflict == TransferConflict::Merge {
                assert_eq!(fs::read(destination.join("folder/stays.txt"))?, b"keep");
            }
            let moved = events
                .borrow()
                .iter()
                .any(|event| matches!(event, OperationEvent::ItemMoved { .. }));
            if flushed {
                assert!(
                    matches!(
                        terminal_transfer(&events.borrow()),
                        Some(OperationEvent::Pasted { .. })
                    ),
                    "{row}: {:?}",
                    events.borrow()
                );
                assert!(!source.exists(), "{row}");
                assert!(moved, "{row}");
            } else {
                assert!(
                    matches!(terminal_transfer(&events.borrow()),
                        Some(OperationEvent::TransferFailed { message, completed_locations, .. })
                        if message.contains("writes may still be pending")
                            && completed_locations.is_empty()),
                    "{row}: {:?}",
                    events.borrow()
                );
                assert_eq!(fs::read(source_dir.join(incoming))?, b"incoming", "{row}");
                assert!(!moved, "{row}");
            }
        }
    }
    Ok(())
}

#[test]
fn same_filesystem_rename_is_not_held_for_the_removable_flush() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let destination = root.path().join("usb");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&destination)?;
    let source = source_dir.join("notes.txt");
    fs::write(&source, b"renamed")?;
    let during = watch_source_during_sync(&source);
    let _guard = RemovableFlushGuard::install(root.path(), Some(io::ErrorKind::Other));

    let (_operation, events) =
        paste_move(96, &destination, &source, TransferConflict::FailIfExists);
    pump_until_transfer(&events);
    let seen = during.lock().unwrap_or_else(|error| error.into_inner());
    assert!(
        !seen.is_empty() && seen.iter().all(|exists| !exists),
        "a same-filesystem rename finishes before the removable flush starts: {seen:?}"
    );

    assert!(matches!(
        terminal_transfer(&events.borrow()),
        Some(OperationEvent::TransferFailed { .. })
    ));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, OperationEvent::Pasted { .. }))
    );
    assert!(!source.exists());
    Ok(())
}

#[test]
fn same_filesystem_replacing_move_renames_without_waiting_for_the_flush()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let destination = root.path().join("usb");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&destination)?;
    let source = source_dir.join("notes.txt");
    fs::write(&source, b"incoming")?;
    fs::write(destination.join("notes.txt"), b"old")?;
    let during = watch_source_during_sync(&source);
    let _guard = RemovableFlushGuard::install(root.path(), Some(io::ErrorKind::Other));
    let _quirks = FsQuirks::install(
        root.path(),
        Quirks {
            trash: super::fs_quirks::Trash::Unsupported,
            ..Quirks::default()
        },
    );

    let (_operation, events) =
        paste_move(97, &destination, &source, TransferConflict::ReplaceExisting);
    pump_until_transfer(&events);
    let seen = during.lock().unwrap_or_else(|error| error.into_inner());
    assert!(
        !seen.is_empty() && seen.iter().all(|exists| !exists),
        "the source was renamed before the removable flush started: {seen:?}"
    );

    assert!(!source.exists());
    assert_eq!(fs::read(destination.join("notes.txt"))?, b"incoming");
    assert_eq!(fs::read_dir(&destination)?.count(), 1);
    Ok(())
}

#[test]
fn keep_both_move_renumbers_when_its_name_is_taken_meanwhile() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for across_volumes in [false, true] {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir_all(&sources)?;
        fs::create_dir_all(&destination)?;
        let source = sources.join("report.txt");
        fs::write(&source, b"incoming")?;
        fs::write(destination.join("report.txt"), b"existing")?;
        let taken = destination.join("report (1).txt");
        let racer = |taken: PathBuf| -> super::fs_quirks::Racer {
            Box::new(move || {
                if fs::symlink_metadata(&taken).is_err() {
                    fs::write(&taken, b"taken meanwhile").expect("take the name");
                }
            })
        };
        let racing = || Quirks {
            before: vec![
                (Op::Rename, "report (1).txt", 0, racer(taken.clone())),
                (Op::LinkFd, "report (1).txt", 0, racer(taken.clone())),
            ],
            ..Quirks::default()
        };
        let _scopes = if across_volumes {
            vec![
                FsQuirks::install(&sources, Quirks::default()),
                FsQuirks::install(&destination, racing()),
            ]
        } else {
            vec![FsQuirks::install(root.path(), racing())]
        };

        let events = run_paste(PasteRequest {
            id: OperationRequestId(78),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::KeepBoth,
                target_name: None,
            }],
            move_sources: true,
        });

        let moved = destination.join("report (2).txt");
        assert_eq!(
            item_moves(&events),
            [(Location::local(&source), Location::local(&moved))],
            "across volumes: {across_volumes}"
        );
        assert_eq!(fs::read(&taken)?, b"taken meanwhile");
        assert_eq!(fs::read(&moved)?, b"incoming");
        assert!(!source.exists());
    }
    Ok(())
}

fn reported(events: &RefCell<Vec<OperationEvent>>) -> TransferReport {
    let mut reported = TransferReport::default();
    for event in events.borrow().iter() {
        if let OperationEvent::ItemsReported { report, .. } = event {
            reported.extend(report.clone());
        }
    }
    reported
}

fn kept(location: PathBuf, cause: KeptCause) -> KeptItem {
    KeptItem {
        location: Location::local(location),
        cause,
    }
}

fn destination_for(
    root: &Path,
    conflict: TransferConflict,
) -> Result<(PathBuf, FsQuirks), Box<dyn Error>> {
    let destination = root.join("destination");
    fs::create_dir_all(&destination)?;
    match conflict {
        TransferConflict::Merge => fs::create_dir(destination.join("folder"))?,
        TransferConflict::ReplaceExisting => {
            fs::create_dir(destination.join("folder"))?;
            fs::write(destination.join("folder/old.txt"), b"old")?;
        }
        _ => {}
    }
    let quirks = Quirks {
        trash: super::fs_quirks::Trash::Unsupported,
        ..Quirks::default()
    };
    let scope = FsQuirks::install(&destination, quirks);
    Ok((destination, scope))
}

const MOVES: [TransferConflict; 3] = [
    TransferConflict::FailIfExists,
    TransferConflict::Merge,
    TransferConflict::ReplaceExisting,
];

#[test]
fn a_move_leaves_special_files_it_cannot_copy_at_the_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for conflict in MOVES {
        let root = tempfile::tempdir()?;
        let source_dir = root.path().join("source");
        let folder = source_dir.join("folder");
        fs::create_dir_all(&folder)?;
        fs::write(folder.join("notes.txt"), b"keep-me")?;
        let _socket = std::os::unix::net::UnixListener::bind(folder.join("socket"))?;
        let _source = FsQuirks::install(&source_dir, Quirks::default());
        let (destination, _destination) = destination_for(root.path(), conflict)?;

        let (_operation, events) = paste_move(91, &destination, &folder, conflict);
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{conflict:?}: {:?}",
            events.borrow()
        );
        assert_eq!(
            names(&destination.join("folder"))?,
            ["notes.txt"],
            "{conflict:?}"
        );
        assert_eq!(names(&folder)?, ["socket"], "{conflict:?}");
        assert_eq!(
            reported(&events),
            TransferReport {
                kept: vec![kept(
                    folder.join("socket"),
                    KeptCause::Skipped(SpecialKind::Socket)
                )],
                ..TransferReport::default()
            },
            "{conflict:?}"
        );
    }
    Ok(())
}

fn folder_with_links(folder: &Path) -> io::Result<()> {
    fs::create_dir_all(folder.join("sub"))?;
    fs::write(folder.join("notes.txt"), b"notes")?;
    fs::write(folder.join("sub/b.txt"), b"b")?;
    std::os::unix::fs::symlink("notes.txt", folder.join("link"))?;
    std::os::unix::fs::symlink("../notes.txt", folder.join("sub/inner"))
}

fn links_kept(folder: &Path) -> Vec<KeptItem> {
    ["link", "sub/inner"]
        .map(|link| kept(folder.join(link), KeptCause::Skipped(SpecialKind::Symlink)))
        .to_vec()
}

#[test]
fn a_move_keeps_the_links_it_leaves_out_at_the_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for conflict in MOVES {
        let root = tempfile::tempdir()?;
        let source_dir = root.path().join("source");
        let folder = source_dir.join("folder");
        folder_with_links(&folder)?;
        let _source = FsQuirks::install(&source_dir, Quirks::default());
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        // A merge leaves what it would replace with a link it leaves out.
        let there = match conflict {
            TransferConflict::Merge => Some("link"),
            TransferConflict::ReplaceExisting => Some("old.txt"),
            _ => None,
        };
        if let Some(name) = there {
            fs::create_dir(destination.join("folder"))?;
            fs::write(destination.join("folder").join(name), b"earlier")?;
        }
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                trash: super::fs_quirks::Trash::Unsupported,
                ..Quirks::vfat()
            },
        );

        let (_operation, events) = paste_move(93, &destination, &folder, conflict);
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{conflict:?}: {:?}",
            events.borrow()
        );
        let moved = destination.join("folder");
        let mut expected = vec!["notes.txt", "sub"];
        if conflict == TransferConflict::Merge {
            expected.insert(0, "link");
            assert_eq!(fs::read(moved.join("link"))?, b"earlier");
        }
        assert_eq!(names(&moved)?, expected, "{conflict:?}");
        assert_eq!(names(&moved.join("sub"))?, ["b.txt"], "{conflict:?}");
        assert_eq!(names(&folder)?, ["link", "sub"], "{conflict:?}");
        assert_eq!(names(&folder.join("sub"))?, ["inner"], "{conflict:?}");
        let mut kept = reported(&events).kept;
        kept.sort_by(|left, right| left.location.compare(&right.location));
        assert_eq!(kept, links_kept(&folder), "{conflict:?}");
    }
    Ok(())
}

#[test]
fn a_link_moved_within_a_share_that_refuses_new_links_is_moved() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let outside = root.path().join("outside");
    let share = root.path().join("share");
    for folder in [&outside, &share.join("from"), &share.join("to")] {
        fs::create_dir_all(folder)?;
    }
    std::os::unix::fs::symlink("target", outside.join("refused"))?;
    // As the server made it.
    std::os::unix::fs::symlink("target", share.join("from/link"))?;
    let _share = FsQuirks::install(&share, Quirks::cifs());
    let (_operation, events) = paste_move(
        98,
        &share,
        &outside.join("refused"),
        TransferConflict::FailIfExists,
    );
    pump_until_transfer(&events);
    assert_eq!(names(&share)?, ["from", "to"]);

    let (_operation, events) = paste_move(
        99,
        &share.join("to"),
        &share.join("from/link"),
        TransferConflict::FailIfExists,
    );
    pump_until_transfer(&events);

    assert!(
        matches!(
            terminal_transfer(&events.borrow()),
            Some(OperationEvent::Pasted { .. })
        ),
        "{:?}",
        events.borrow()
    );
    assert_eq!(fs::read_link(share.join("to/link"))?, Path::new("target"));
    assert!(names(&share.join("from"))?.is_empty());
    Ok(())
}

#[test]
fn a_move_through_gio_keeps_the_links_it_leaves_out_at_the_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    folder_with_links(&folder)?;
    let stand_in = root.path().join("remote");
    fs::create_dir(&stand_in)?;
    let _stand_in = FsQuirks::install(&stand_in, Quirks::vfat());
    let target = gio::File::for_uri("strata-test-remote:///folder");
    let copied_to = stand_in.join("folder");
    let remote_copy: super::super::RemoteCopy =
        Rc::new(move |source, target, _, cancellable, left| {
            let copied_to = copied_to.clone();
            Box::pin(async move {
                super::super::copy_recursively_with_progress(
                    source,
                    gio::File::for_path(copied_to),
                    super::super::CopyInto::New,
                    cancellable,
                    super::super::CopyRecord::Nothing,
                    None,
                    super::super::GioNames::new(fixing(), left),
                )
                .await?;
                Ok(target)
            })
        });
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let progress = TransferProgressTracker::new(
        OperationRequestId(1),
        None,
        None,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    glib::MainContext::default().block_on(super::super::move_local_with_progress(
        gio::File::for_path(&folder),
        target,
        Taken::Fail,
        gio::Cancellable::new(),
        super::super::Copying {
            progress: Some(progress),
            names: fixing(),
            ..Default::default()
        },
        super::super::attempt_move(None),
        remote_copy,
    ))?;

    // GIO alone would have made the links, as the stand-in's own filesystem holds them.
    assert_eq!(names(&stand_in.join("folder"))?, ["notes.txt", "sub"]);
    assert_eq!(names(&stand_in.join("folder/sub"))?, ["b.txt"]);
    assert_eq!(names(&folder)?, ["link", "sub"]);
    assert_eq!(names(&folder.join("sub"))?, ["inner"]);
    let mut kept = reported(&events).kept;
    kept.sort_by(|left, right| left.location.compare(&right.location));
    assert_eq!(kept, links_kept(&folder));
    Ok(())
}

#[test]
fn deleting_a_source_gio_moved_keeps_the_links_it_left_out() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    folder_with_links(&folder)?;
    fs::create_dir(folder.join("other"))?;
    fs::write(folder.join("other/c.txt"), b"c")?;
    let left = ["link", "sub/inner"].map(|link| gio::File::for_path(folder.join(link)));
    let mut owed = super::super::owed::Owed::default();
    owed.owe(super::super::owed::Deletion::Source(
        gio::File::for_path(&folder),
        left.to_vec(),
    ));
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let progress = TransferProgressTracker::new(
        OperationRequestId(1),
        None,
        None,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    glib::MainContext::default()
        .block_on(owed.settle(Some(&progress), &gio::Cancellable::new()))?;

    assert_eq!(names(&folder)?, ["link", "sub"]);
    assert_eq!(names(&folder.join("sub"))?, ["inner"]);
    let mut kept = reported(&events).kept;
    kept.sort_by(|left, right| left.location.compare(&right.location));
    assert_eq!(kept, links_kept(&folder));
    Ok(())
}

#[test]
fn a_move_keeps_source_entries_changed_or_added_after_their_copy() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for conflict in MOVES {
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
        let racer: super::fs_quirks::Racer = Box::new(move || {
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

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{conflict:?}: {:?}",
            events.borrow()
        );
        let moved = destination.join("folder");
        assert_eq!(fs::read(moved.join("a.txt"))?, b"a", "{conflict:?}");
        assert_eq!(fs::read(moved.join("sub/b.txt"))?, b"b", "{conflict:?}");
        assert_eq!(names(&folder)?, ["added.txt", "sub"], "{conflict:?}");
        assert_eq!(fs::read(&changed)?, b"rewritten", "{conflict:?}");
        assert_eq!(fs::read(&added)?, b"added", "{conflict:?}");
        assert_eq!(
            reported(&events).kept,
            [
                kept(changed, KeptCause::Changed),
                kept(folder.clone(), KeptCause::Changed)
            ],
            "{conflict:?}"
        );
    }
    Ok(())
}

#[test]
fn a_file_written_during_its_own_move_stays_at_the_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let rewrite = |path: &Path, contents: &'static [u8]| -> super::fs_quirks::Racer {
        let path = path.to_path_buf();
        Box::new(move || fs::write(&path, contents).expect("rewrite the source"))
    };
    let cases: [(&str, usize, Preset); 4] = [
        ("in a folder", 1, Quirks::default),
        ("in a folder", 2, Quirks::default),
        ("linked", 2, Quirks::nfs3),
        ("selected", 2, Quirks::default),
    ];
    for (kind, rewrites, destination_quirks) in cases {
        let case = format!("{kind}, rewrites: {rewrites}");
        let root = tempfile::tempdir()?;
        let source_dir = root.path().join("source");
        let folder = source_dir.join("folder");
        fs::create_dir_all(&folder)?;
        let db = folder.join("db");
        fs::write(&db, b"first")?;
        let reads = if kind == "linked" {
            fs::hard_link(&db, folder.join("DB"))?;
            "*"
        } else {
            fs::write(folder.join("notes.txt"), b"notes")?;
            "db"
        };
        // Each pass reads the data, then the end of the file.
        let mut before = vec![(Op::Read, reads, 0, rewrite(&db, b"second write"))];
        if rewrites == 2 {
            before.push((Op::Read, reads, 1, rewrite(&db, b"third")));
        }
        let _source = FsQuirks::install(
            &source_dir,
            Quirks {
                before,
                ..Quirks::default()
            },
        );
        let destination = root.path().join("destination");
        fs::create_dir_all(&destination)?;
        let _destination = FsQuirks::install(&destination, destination_quirks());
        let moved = if kind == "selected" { &db } else { &folder };

        let (_operation, events) =
            paste_move(93, &destination, moved, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        let terminal = terminal_transfer(&events.borrow()).cloned();
        if kind == "selected" {
            let Some(OperationEvent::TransferFailed { message, .. }) = terminal else {
                panic!("{case}: {:?}", events.borrow());
            };
            assert_eq!(
                message,
                rust_i18n::t!("%{name} changed while it was being copied", name = "db"),
                "{case}"
            );
            assert!(names(&destination)?.is_empty(), "{case}");
            assert_eq!(fs::read(&db)?, b"third", "{case}");
            continue;
        }
        assert!(
            matches!(terminal, Some(OperationEvent::Pasted { .. })),
            "{case}: {:?}",
            events.borrow()
        );
        let mut left = reported(&events).kept;
        left.sort_by_key(|item| item.location.native_path().map(Path::to_path_buf));
        if rewrites == 1 {
            assert_eq!(
                fs::read(destination.join("folder/db"))?,
                b"second write",
                "{case}"
            );
            assert!(!folder.exists(), "{case}");
            assert!(left.is_empty(), "{case}");
        } else if kind == "linked" {
            assert!(names(&destination.join("folder"))?.is_empty(), "{case}");
            assert_eq!(names(&folder)?, ["DB", "db"], "{case}");
            assert_eq!(fs::read(&db)?, b"third", "{case}");
            let changed = |name| kept(folder.join(name), KeptCause::Changed);
            assert_eq!(left, [changed("DB"), changed("db")], "{case}");
        } else {
            assert_eq!(names(&destination.join("folder"))?, ["notes.txt"], "{case}");
            assert_eq!(names(&folder)?, ["db"], "{case}");
            assert_eq!(fs::read(&db)?, b"third", "{case}");
            assert_eq!(left, [kept(db.clone(), KeptCause::Changed)], "{case}");
        }
    }
    Ok(())
}

#[test]
fn moving_across_volumes_deletes_every_entry_it_copied() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for selected in ["folder", "link", "pipe"] {
        let root = tempfile::tempdir()?;
        let source_dir = root.path().join("source");
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
        let destination = root.path().join("destination");
        fs::create_dir_all(&destination)?;
        let _volumes = separate_volumes(&source_dir, &destination);
        let source = source_dir.join(selected);

        let (_operation, events) =
            paste_move(94, &destination, &source, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{selected}: {:?}",
            events.borrow()
        );
        assert!(fs::symlink_metadata(&source).is_err(), "{selected}");
        assert_eq!(reported(&events), TransferReport::default(), "{selected}");
        let moved = destination.join(selected);
        match selected {
            "folder" => {
                let (a, b) = (
                    fs::metadata(moved.join("a"))?,
                    fs::metadata(moved.join("sub/b"))?,
                );
                assert_eq!((a.ino(), a.nlink()), (b.ino(), 2));
                assert_eq!(fs::read(moved.join("c"))?, b"outside");
                assert_eq!(names(&moved.join("sub"))?, ["b", "link", "pipe"]);
                assert_eq!(fs::metadata(&outside)?.nlink(), 1);
            }
            "link" => assert_eq!(fs::read_link(&moved)?, Path::new("folder/a")),
            _ => assert!(fs::symlink_metadata(&moved)?.file_type().is_fifo()),
        }
    }
    Ok(())
}

#[test]
fn a_name_of_a_linked_file_changed_after_its_other_name_went_stays() -> Result<(), Box<dyn Error>> {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
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
        let root = tempfile::tempdir()?;
        let source_dir = root.path().join("source");
        let folder = source_dir.join("folder");
        fs::create_dir_all(&folder)?;
        fs::write(folder.join("a"), b"linked")?;
        if !hard_linked(&folder.join("a"), &folder.join("b"))? {
            eprintln!("skipped on a base without hard links: {row}");
            continue;
        }
        if !numbers_go && super::fs_quirks::smb_base(fs::File::open(&folder)?.as_fd()) {
            eprintln!(
                "skipped on an SMB base, which refuses to open a link's second name while the first is open: {row}"
            );
            continue;
        }
        let hour_before = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs()
            - 3600;
        set_times(&folder.join("a"), hour_before, 0)?;
        if numbers_go && !a_later_change_time(&folder.join("b"), &folder.join("a"), hour_before)? {
            eprintln!(
                "skipped on a base that reports the modification time as the change time: {row}"
            );
            continue;
        }
        let (deleting, checked) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicUsize::new(0)),
        );
        let rewrite_second = |name: &'static str| -> super::fs_quirks::Racer {
            let (deleting, checked, path, probe) = (
                deleting.clone(),
                checked.clone(),
                folder.join(name),
                root.path().join("probe"),
            );
            Box::new(move || {
                if deleting.load(Ordering::Acquire) && checked.fetch_add(1, Ordering::SeqCst) == 1 {
                    let before = fs::metadata(&path).expect("stat the file");
                    wait_for_a_later_change_time(&probe, &before);
                    fs::write(&path, b"LINKED").expect("rewrite the file");
                    set_times(&path, hour_before, 0).expect("put the times back");
                }
            })
        };
        let went = Arc::new(AtomicBool::new(!numbers_go));
        let (opening, renumbering, linked) = (
            deleting.clone(),
            went.clone(),
            [folder.join("a"), folder.join("b")],
        );
        let deletion_opens: super::fs_quirks::Racer = Box::new(move || {
            if evicted {
                renumbering.fetch_and(
                    linked.iter().all(|path| super::fs_quirks::evict(path)),
                    Ordering::AcqRel,
                );
            }
            opening.store(true, Ordering::Release);
        });
        let mut before = vec![(Op::OpenDir, "folder", 1, deletion_opens)];
        if numbers_go {
            let (at, going) = (source_dir.clone(), went.clone());
            let numbers_went: super::fs_quirks::Racer = Box::new(move || {
                going.store(
                    super::fs_quirks::autodisable_serverino(&at),
                    Ordering::Release,
                );
            });
            before.push((Op::Stat, "folder", 0, numbers_went));
        }
        for name in ["a", "b", "a", "b"] {
            before.push((Op::Stat, name, 0, rewrite_second(name)));
        }
        let quirks = Quirks { before, ..source() };

        let (moved, report, _) = move_across_volumes(&folder, quirks)?;

        assert!(went.load(Ordering::Acquire), "{row}");
        assert_eq!(fs::read(moved.join("a"))?, b"linked", "{row}");
        assert_eq!(fs::read(moved.join("b"))?, b"linked", "{row}");
        assert!(folder.exists(), "{row}");
        let left = names(&folder)?;
        assert_eq!(left.len(), 1, "{row}: {left:?}");
        let second = folder.join(&left[0]);
        assert_eq!(fs::read(&second)?, b"LINKED", "{row}");
        assert_eq!(report.kept, [kept(second, KeptCause::Changed)], "{row}");
    }
    Ok(())
}

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

fn move_across_volumes(
    folder: &Path,
    quirks: Quirks,
) -> Result<(PathBuf, TransferReport, Vec<OperationEvent>), Box<dyn Error>> {
    move_across_volumes_logged(folder, quirks)
        .map(|(moved, report, events, _)| (moved, report, events))
}

type LoggedMove = (
    PathBuf,
    TransferReport,
    Vec<OperationEvent>,
    Vec<super::fs_quirks::Call>,
);

fn move_across_volumes_logged(folder: &Path, quirks: Quirks) -> Result<LoggedMove, Box<dyn Error>> {
    let source_dir = folder.parent().ok_or("a folder in a source volume")?;
    let destination = source_dir.with_file_name("destination");
    fs::create_dir_all(&destination)?;
    let source = FsQuirks::install(source_dir, quirks);
    let _destination = FsQuirks::install(&destination, Quirks::default());

    let (_operation, events) = paste_move(97, &destination, folder, TransferConflict::FailIfExists);
    pump_until_transfer(&events);

    assert!(
        matches!(
            terminal_transfer(&events.borrow()),
            Some(OperationEvent::Pasted { .. })
        ),
        "{:?}",
        events.borrow()
    );
    let report = reported(&events);
    let name = folder.file_name().ok_or("a named item")?;
    Ok((
        destination.join(name),
        report,
        events.take(),
        source.calls(),
    ))
}

fn rewrite(path: &Path, contents: &'static [u8]) -> super::fs_quirks::Racer {
    let path = path.to_path_buf();
    Box::new(move || fs::write(&path, contents).expect("rewrite the source"))
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
        let racer: super::fs_quirks::Racer = Box::new(move || {
            let before = fs::metadata(&path).expect("stat the source");
            wait_for_a_later_change_time(&probe, &before);
            fs::write(&path, b"B2").expect("rewrite the source");
            let modified = rustix::fs::Timespec {
                tv_sec: before.mtime(),
                tv_nsec: before.mtime_nsec(),
            };
            rustix::fs::utimensat(
                rustix::fs::CWD,
                &path,
                &rustix::fs::Timestamps {
                    last_access: modified,
                    last_modification: modified,
                },
                rustix::fs::AtFlags::empty(),
            )
            .expect("put the times back");
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

/// A rewrite within one change-time tick of `than` (10 ms on exFAT) would keep
/// its change time.
fn wait_for_a_later_change_time(probe: &Path, than: &fs::Metadata) {
    loop {
        fs::write(probe, b"probe").expect("write the probe");
        let stamped = fs::metadata(probe).expect("stat the probe");
        if (stamped.ctime(), stamped.ctime_nsec()) > (than.ctime(), than.ctime_nsec()) {
            break;
        }
    }
    fs::remove_file(probe).expect("remove the probe");
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
        let _kernel = KernelGuard::set(Some(kernel));
        // Before the kernel's coarse clock, which may run a tick behind.
        let tick = SystemTime::now() - Duration::from_secs(1);
        let _clock = ClockGuard::set(tick);
        let root = tempfile::tempdir()?;
        let folder = root.path().join("source/folder");
        fs::create_dir_all(&folder)?;
        let db = folder.join("db");
        fs::write(&db, b"first")?;
        let (before, expected): (Vec<_>, &[u8]) = if row == "same size" {
            let hour_before = tick.duration_since(SystemTime::UNIX_EPOCH)?.as_secs() - 3600;
            set_times(&db, hour_before, 0)?;
            (vec![(Op::Read, "db", 1, rewrite(&db, b"other"))], b"other")
        } else {
            (
                vec![
                    (Op::Read, "db", 0, rewrite(&db, b"a much longer version")),
                    (Op::Read, "db", 0, rewrite(&db, b"short")),
                ],
                b"short",
            )
        };
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
            let transferred = events.iter().rev().find_map(|event| match event {
                OperationEvent::TransferProgress {
                    transferred_bytes, ..
                } => Some(*transferred_bytes),
                _ => None,
            });
            assert_eq!(transferred, Some(5), "{row}");
        }
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
        let hour_before = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs()
            - 3600;
        set_times(&file, hour_before, 0)?;
        let quirks = Quirks {
            mount_type: Some(("fuse.lxcfs", "rw")),
            virtual_sizes: true,
            ..Quirks::default()
        };

        let item = if in_folder { &folder } else { &file };
        let (moved, report, _) = move_across_volumes(item, quirks)?;

        assert_eq!(
            report,
            TransferReport::default(),
            "in a folder: {in_folder}"
        );
        assert!(!file.exists(), "in a folder: {in_folder}");
        let moved = if in_folder {
            moved.join("cpuinfo")
        } else {
            moved
        };
        assert_eq!(
            fs::read(moved)?,
            b"processor : 0\n",
            "in a folder: {in_folder}"
        );
    }
    Ok(())
}

#[test]
fn moves_off_mounts_without_stable_inodes_keep_only_what_changed() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("source/folder");
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
    let hour_before = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs()
        - 3600;
    for file in ["a.txt", "sub/b.txt", "x", "m", "p"] {
        set_times(&folder.join(file), hour_before, 0)?;
    }
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
    .map(|path| folder.join(path));
    let changed = folder.clone();
    // The kernel evicts every entry between the copy and the deletion, while
    // files are rewritten. cifs counts links, which sshfs does not.
    let racer: super::fs_quirks::Racer = Box::new(move || {
        fs::write(changed.join("sub/b.txt"), b"rewritten").expect("rewrite b");
        fs::write(changed.join("m"), b"M").expect("rewrite m");
        let resized = changed.join("p");
        let before = fs::metadata(&resized).expect("stat p");
        fs::write(&resized, b"longer").expect("rewrite p");
        let modified = rustix::fs::Timespec {
            tv_sec: before.mtime(),
            tv_nsec: before.mtime_nsec(),
        };
        rustix::fs::utimensat(
            rustix::fs::CWD,
            &resized,
            &rustix::fs::Timestamps {
                last_access: modified,
                last_modification: modified,
            },
            rustix::fs::AtFlags::empty(),
        )
        .expect("put the times back");
        for path in &evicted {
            super::fs_quirks::renumbered(path);
        }
    });
    let quirks = Quirks {
        mount_type: Some(("cifs", "rw,vers=3.1.1")),
        before: vec![(Op::OpenDir, "folder", 1, racer)],
        ..Quirks::default()
    };

    let (moved, report, _) = move_across_volumes(&folder, quirks)?;

    assert_eq!(fs::read(moved.join("sub/b.txt"))?, b"b");
    assert_eq!(fs::read(moved.join("sub/y"))?, b"x");
    assert_eq!(names(&folder)?, ["m", "p", "sub"]);
    assert_eq!(names(&folder.join("sub"))?, ["b.txt", "n", "q"]);
    let mut left = report.kept;
    left.sort_by_key(|item| item.location.native_path().map(Path::to_path_buf));
    let expected = ["m", "p", "sub/b.txt", "sub/n", "sub/q"]
        .map(|path| kept(folder.join(path), KeptCause::Changed));
    assert_eq!(left, expected);
    Ok(())
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
        let at = folder.clone();
        let (before, after) = match row {
            "swapped" => {
                let racer: super::fs_quirks::Racer = Box::new(move || {
                    fs::remove_dir_all(at.join("sub")).expect("remove the folder");
                    fs::write(at.join("sub"), b"a file now").expect("put a file there");
                });
                (vec![(Op::OpenDir, "sub", 1, racer)], Vec::new())
            }
            "removed" => {
                let racer: super::fs_quirks::Racer = Box::new(move || {
                    fs::remove_file(at.join("a.txt")).expect("remove the file");
                });
                (vec![(Op::OpenDir, "folder", 1, racer)], Vec::new())
            }
            _ => {
                let racer: super::fs_quirks::Racer = Box::new(move || {
                    fs::rename(&at, at.with_file_name("emptied")).expect("move it");
                    fs::create_dir(&at).expect("make another folder");
                });
                (Vec::new(), vec![(Op::Unlink, "a.txt", 0, racer)])
            }
        };
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
                    let racer: super::fs_quirks::Racer = Box::new(move || {
                        for path in &evicted {
                            super::fs_quirks::renumbered(path);
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
                    let racer: super::fs_quirks::Racer = Box::new(move || {
                        let numbers_went =
                            !turned_off || super::fs_quirks::autodisable_serverino(&at);
                        let happened = numbers_went
                            && match meanwhile {
                                Meanwhile::Renumbered => {
                                    evicted.iter().all(|path| super::fs_quirks::evict(path))
                                }
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

    assert_eq!(super::names(&destination)?, names);
    assert!(super::names(&sources)?.is_empty());
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

#[derive(Clone, Copy, Debug)]
enum SourceMeanwhile {
    Closes,
    IsSwapped,
    StaysOpen(Option<&'static str>),
    DeletedBefore { closes: bool },
}

#[test]
fn a_moved_folder_goes_once_another_program_closes_its_file_within_the_mount_s_wait()
-> Result<(), Box<dyn Error>> {
    use SourceMeanwhile::*;
    let _serial = serial()?;
    let rows: [(&str, super::fs_quirks::Preset, SourceMeanwhile); 9] = [
        ("nfs42", Quirks::nfs42, Closes),
        ("nfs42", Quirks::nfs42, IsSwapped),
        ("nfs42", Quirks::nfs42, StaysOpen(None)),
        ("nfs42", Quirks::nfs42, StaysOpen(Some("Text Editor"))),
        ("cifs-posix", Quirks::cifs_posix, StaysOpen(None)),
        (
            "cifs-posix",
            Quirks::cifs_posix,
            StaysOpen(Some("Text Editor")),
        ),
        ("nfs42", Quirks::nfs42, DeletedBefore { closes: false }),
        ("nfs42", Quirks::nfs42, DeletedBefore { closes: true }),
        ("sshfs", Quirks::sshfs, DeletedBefore { closes: false }),
    ];
    for (mount, preset, meanwhile) in rows {
        let row = format!("{mount}, {meanwhile:?}");
        let _clock = ClockGuard::set(SystemTime::now());
        let root = tempfile::tempdir()?;
        let source_dir = root.path().join("source");
        let folder = source_dir.join("folder");
        let moved = source_dir.join("folder.moved");
        fs::create_dir_all(&folder)?;
        fs::write(folder.join("open.txt"), b"open elsewhere")?;
        fs::write(folder.join("other.txt"), b"other")?;
        let held = Arc::new(std::sync::Mutex::new(None));
        let release = held.clone();
        let (live, aside) = (folder.clone(), moved.clone());
        let racer: Option<super::fs_quirks::Racer> = match meanwhile {
            IsSwapped => Some(Box::new(move || {
                fs::rename(&live, &aside).expect("move the folder");
                fs::create_dir(&live).expect("make another folder");
            })),
            Closes | DeletedBefore { closes: true } => Some(Box::new(move || {
                drop(release.lock().expect("the hold").take());
            })),
            StaysOpen(_) | DeletedBefore { closes: false } => None,
        };
        let holder = match meanwhile {
            StaysOpen(Some(program)) => Some(Holder::Program(program.to_owned())),
            _ => None,
        };
        let _source = FsQuirks::install(
            &source_dir,
            Quirks {
                after: racer
                    .map(|racer| (Op::Unlink, "folder", 0, racer))
                    .into_iter()
                    .collect(),
                holder: holder.clone(),
                ..preset()
            },
        );
        *held.lock().map_err(|_| "the hold")? = Some(hold(&folder.join("open.txt")));
        if let DeletedBefore { .. } = meanwhile {
            let dir = open_local_parent_directory(&folder)?;
            super::super::fs_ops::fs().unlink(
                super::super::fs_ops::Entry::new(&dir, OsStr::new("open.txt")),
                rustix::fs::AtFlags::empty(),
            )?;
        }
        let destination = root.path().join("destination");
        fs::create_dir_all(&destination)?;
        let _destination = FsQuirks::install(&destination, Quirks::default());

        let (_operation, events) =
            paste_move(1590, &destination, &folder, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{row}"
        );
        let copied = destination.join("folder");
        if let DeletedBefore { .. } = meanwhile {
            assert_eq!(names(&copied)?, ["other.txt"], "{row}");
        } else {
            assert_eq!(
                fs::read(copied.join("open.txt"))?,
                b"open elsewhere",
                "{row}"
            );
        }
        let report = reported(&events);
        match meanwhile {
            Closes | DeletedBefore { closes: true } => {
                assert!(!folder.exists(), "{row}");
                assert!(report.kept.is_empty(), "{row}: {:?}", report.kept);
            }
            IsSwapped => {
                assert_eq!(
                    report.kept,
                    [kept(folder.clone(), KeptCause::Changed)],
                    "{row}"
                );
                assert!(names(&folder)?.is_empty(), "{row}");
                assert!(moved.is_dir(), "{row}");
            }
            DeletedBefore { closes: false } => {
                assert_eq!(
                    report.kept,
                    [kept(folder.clone(), KeptCause::StillOpen(None))],
                    "{row}"
                );
                let [leftover] = names(&folder)?.try_into().map_err(|_| "one leftover")?;
                assert!(
                    crate::model::hidden_names::foreign_leftover(OsStr::new(&leftover)).is_some(),
                    "{row}: {leftover}"
                );
            }
            StaysOpen(_) => {
                let cause = match (mount, holder) {
                    ("cifs-posix", None) => KeptCause::Undeletable(Errno::ACCESS.raw_os_error()),
                    (_, holder) => KeptCause::StillOpen(holder),
                };
                assert_eq!(report.kept, [kept(folder.clone(), cause)], "{row}");
                assert!(folder.is_dir(), "{row}");
            }
        }
        drop(held.lock().map_err(|_| "the hold")?.take());
    }
    Ok(())
}

#[test]
fn a_move_cancelled_while_deleting_its_sources_reports_what_stays() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let folder = source_dir.join("folder");
    fs::create_dir_all(&folder)?;
    fs::write(folder.join("a.txt"), b"a")?;
    let _socket = std::os::unix::net::UnixListener::bind(folder.join("socket"))?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let cancellable = gio::Cancellable::new();
    let cancel = cancellable.clone();
    let _source = FsQuirks::install(
        &source_dir,
        Quirks {
            before: vec![(Op::Unlink, "*", 0, Box::new(move || cancel.cancel()))],
            ..Quirks::default()
        },
    );
    let _destination = FsQuirks::install(&destination, Quirks::default());
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let progress = TransferProgressTracker::new(
        OperationRequestId(1),
        None,
        None,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    let copying = super::super::Copying {
        progress: Some(progress),
        names: super::super::NamePolicy::default(),
        ..Default::default()
    };

    let moved = glib::MainContext::default().block_on(super::super::move_local_with_progress(
        gio::File::for_path(&folder),
        gio::File::for_path(destination.join("folder")),
        Taken::Fail,
        cancellable,
        copying.clone(),
        super::super::attempt_move(None),
        super::super::remote_copy(copying),
    ));

    assert!(
        moved.is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)),
        "the move stops while it deletes its sources"
    );
    assert_eq!(
        reported(&events).kept,
        [kept(
            folder.join("socket"),
            KeptCause::Skipped(SpecialKind::Socket)
        )]
    );
    Ok(())
}

#[test]
fn moving_to_a_remote_target_keeps_a_source_entry_changed_during_the_copy()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    fs::create_dir_all(&folder)?;
    fs::write(folder.join("a.txt"), b"a")?;
    fs::write(folder.join("b.txt"), b"b")?;
    let stand_in = root.path().join("remote");
    let target = gio::File::for_uri("strata-test-remote:///folder");
    assert!(!target.is_native());
    let (changed, copied_to) = (folder.join("b.txt"), stand_in.clone());
    let remote_copy: super::super::RemoteCopy =
        Rc::new(move |source, target, _, cancellable, _| {
            let (changed, copied_to) = (changed.clone(), copied_to.clone());
            Box::pin(async move {
                copy_recursively(source, gio::File::for_path(copied_to), false, cancellable)
                    .await?;
                fs::write(&changed, b"changed").map_err(io_error)?;
                Ok(target)
            })
        });
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let progress = TransferProgressTracker::new(
        OperationRequestId(1),
        None,
        None,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    let moved = glib::MainContext::default().block_on(super::super::move_local_with_progress(
        gio::File::for_path(&folder),
        target.clone(),
        Taken::Fail,
        gio::Cancellable::new(),
        super::super::Copying {
            progress: Some(progress),
            names: super::super::NamePolicy::default(),
            ..Default::default()
        },
        super::super::attempt_move(None),
        remote_copy,
    ))?;

    assert!(moved.equal(&target));
    assert_eq!(fs::read(stand_in.join("a.txt"))?, b"a");
    assert_eq!(fs::read(stand_in.join("b.txt"))?, b"b");
    assert_eq!(names(&folder)?, ["b.txt"]);
    assert_eq!(
        reported(&events).kept,
        [kept(folder.join("b.txt"), KeptCause::Changed)]
    );
    Ok(())
}

#[test]
fn moving_from_a_share_to_a_remote_target_right_after_a_paste_there_deletes_its_sources()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let _clock = ClockGuard::set(SystemTime::now());
    let root = tempfile::tempdir()?;
    let origin = root.path().join("origin");
    let share = root.path().join("share");
    fs::create_dir_all(origin.join("folder"))?;
    fs::write(origin.join("folder/a.txt"), b"a")?;
    fs::create_dir(&share)?;
    let _share = FsQuirks::install(&share, Quirks::cifs_posix());
    run_paste(PasteRequest {
        id: OperationRequestId(1594),
        destination: Location::local(&share),
        items: vec![PasteItem {
            source: Location::local(origin.join("folder")),
            conflict: TransferConflict::FailIfExists,
            target_name: None,
        }],
        move_sources: false,
    });
    land_deferred_closes(&share)?;
    let folder = share.join("folder");
    let stand_in = root.path().join("remote");
    let target = gio::File::for_uri("strata-test-remote:///folder");
    let copied_to = stand_in.clone();
    let remote_copy: super::super::RemoteCopy =
        Rc::new(move |source, target, _, cancellable, _| {
            let copied_to = copied_to.clone();
            Box::pin(async move {
                copy_recursively(source, gio::File::for_path(copied_to), false, cancellable)
                    .await?;
                Ok(target)
            })
        });
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let progress = TransferProgressTracker::new(
        OperationRequestId(1),
        None,
        None,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    glib::MainContext::default().block_on(super::super::move_local_with_progress(
        gio::File::for_path(&folder),
        target,
        Taken::Fail,
        gio::Cancellable::new(),
        super::super::Copying {
            progress: Some(progress),
            ..Default::default()
        },
        super::super::attempt_move(None),
        remote_copy,
    ))?;

    assert_eq!(fs::read(stand_in.join("a.txt"))?, b"a");
    assert_eq!(reported(&events).kept, []);
    assert!(names(&share)?.is_empty());
    Ok(())
}

#[test]
fn a_moved_source_stays_when_its_copy_is_gone_before_the_deletion() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Replaced {
        No,
        Yes,
        /// Same size, written a tick after the source: how a file reusing the
        /// copy's inode within its birth tick looks on ntfs3.
        InPlace,
        /// So within the tick of a coarse clock that stamped the source too.
        InPlaceWithinTheTick,
        /// So within a tick of ntfs3's clock, which stays coarse after Linux
        /// 6.13, from a source with multigrain times.
        InPlaceWithinTheTickOnNtfs3,
    }
    let rows = [
        (TransferConflict::FailIfExists, None),
        (TransferConflict::ReplaceExisting, None),
        (TransferConflict::FailIfExists, Some(Op::LinkFd)),
        (TransferConflict::FailIfExists, Some(Op::Rename)),
        (TransferConflict::ReplaceExisting, Some(Op::Exchange)),
    ]
    .into_iter()
    .flat_map(|(conflict, published)| {
        [Replaced::No, Replaced::Yes].map(|replaced| (conflict, published, replaced))
    })
    .chain(
        [
            Replaced::InPlace,
            Replaced::InPlaceWithinTheTick,
            Replaced::InPlaceWithinTheTickOnNtfs3,
        ]
        .map(|replaced| (TransferConflict::FailIfExists, None, replaced)),
    );
    for (conflict, published, replaced) in rows {
        let row = format!("{conflict:?}, published: {published:?}, replaced: {replaced:?}");
        let on_ntfs3 = replaced == Replaced::InPlaceWithinTheTickOnNtfs3;
        let within_the_tick = on_ntfs3 || replaced == Replaced::InPlaceWithinTheTick;
        // Before the kernel's coarse clock, which may run a tick behind.
        let tick = SystemTime::now() - Duration::from_secs(1);
        let _clock = within_the_tick.then(|| ClockGuard::set(tick));
        let kernel = if on_ntfs3 { (6, 13, 0) } else { (6, 12, 0) };
        let _kernel = within_the_tick.then(|| KernelGuard::set(Some(kernel)));
        let coarse_clock = within_the_tick.then_some(tick);
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let source = sources.join("notes.txt");
        fs::write(&source, b"notes")?;
        if replaced == Replaced::InPlace {
            let hour_before = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
            set_times(&source, hour_before.as_secs() - 3600, 0)?;
        }
        if conflict == TransferConflict::ReplaceExisting {
            fs::write(destination.join("notes.txt"), b"old")?;
        }
        let copy = destination.join("notes.txt");
        let racer: super::fs_quirks::Racer = Box::new(move || {
            if matches!(replaced, Replaced::No | Replaced::Yes) {
                fs::remove_file(&copy).expect("remove the copy");
            }
            if replaced != Replaced::No {
                fs::write(&copy, b"other").expect("put another file there");
            }
        });
        let _sources = FsQuirks::install(
            &sources,
            Quirks {
                coarse_clock,
                ..Quirks::ext4()
            },
        );
        let mut quirks = Quirks {
            trash: super::fs_quirks::Trash::Unsupported,
            coarse_clock,
            ..if on_ntfs3 {
                Quirks::ntfs3()
            } else {
                Quirks::ext4()
            }
        };
        if let Some(op) = published {
            quirks.after.push((op, "notes.txt", 0, racer));
            if op == Op::Rename {
                quirks.tmpfile = Support::Rejected(Errno::OPNOTSUPP);
            }
        } else {
            quirks.before.push((Op::SyncFs, "*", 0, racer));
        }
        let _destination = FsQuirks::install(&destination, quirks);

        let (_operation, events) = paste_move(98, &destination, &source, conflict);
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{row}: {:?}",
            events.borrow()
        );
        assert_eq!(fs::read(&source).ok(), Some(b"notes".to_vec()), "{row}");
        assert_eq!(
            reported(&events).kept,
            [kept(source.clone(), KeptCause::CopyGone)],
            "{row}"
        );
    }
    Ok(())
}

#[test]
fn a_source_is_deleted_once_its_copy_was_stamped_again_at_close() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // A FUSE writeback cache stamps the close time, within the 2 s of a
    // recent source's that the times check lets stand. An NFSv4.2 write
    // delegation stamps any copy, which gets the source's times again after
    // the close, by name where its owner may neither read nor write it.
    let fuse_writeback = || Quirks {
        times_reset_at_close: true,
        ..Quirks::sshfs()
    };
    let rows: [(&str, Preset, bool, Option<u32>, bool); 4] = [
        ("FUSE writeback cache", fuse_writeback, false, None, true),
        ("NFSv4.2", Quirks::nfs42, false, None, false),
        ("NFSv4.2, inside a folder", Quirks::nfs42, true, None, false),
        (
            "NFSv4.2, unreadable",
            Quirks::nfs42,
            false,
            Some(0o044),
            false,
        ),
    ];
    for (row, preset, in_folder, mode, recent) in rows {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir_all(sources.join("folder"))?;
        fs::create_dir(&destination)?;
        let source = sources.join("folder/notes.txt");
        fs::write(&source, b"notes")?;
        if !recent {
            let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
            set_times(&source, now.as_secs() - 3600, 0)?;
        }
        let _sources = FsQuirks::install(
            &sources,
            Quirks {
                reported_mode: mode.map(|mode| ("notes.txt", mode)),
                ..Quirks::default()
            },
        );
        let _destination = FsQuirks::install(&destination, preset());
        let (item, moved) = if in_folder {
            (sources.join("folder"), destination.join("folder/notes.txt"))
        } else {
            (source.clone(), destination.join("notes.txt"))
        };

        let (_operation, events) =
            paste_move(98, &destination, &item, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        assert!(!item.exists(), "{row}: {:?}", reported(&events).kept);
        fs::set_permissions(&moved, fs::Permissions::from_mode(0o644))?;
        assert_eq!(fs::read(moved)?, b"notes", "{row}");
    }
    Ok(())
}

#[test]
fn a_copy_replaced_before_its_times_are_settled_keeps_its_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // Outside sshfs's 3 s window, so the copy of a recent source is not
    // compared by content.
    let fuse_daemon = || Quirks {
        mount_type: Some(("fuse.rclone", "rw,user_id=1000,group_id=1000")),
        ..Quirks::sshfs()
    };
    let rows: [(&str, Preset, u64, u32); 3] = [
        ("times set again", Quirks::sshfs, 3600, 0o644),
        ("times set again by name", Quirks::sshfs, 3600, 0o044),
        ("times let stand", fuse_daemon, 1, 0o644),
    ];
    for (row, preset, age, mode) in rows {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        let source = sources.join("folder");
        fs::create_dir_all(&source)?;
        fs::create_dir(&destination)?;
        let notes = source.join("notes.txt");
        fs::write(&notes, b"notes")?;
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
        set_times(&notes, now.as_secs() - age, 500_000_000)?;
        let replaced = Arc::new(AtomicBool::new(false));
        let replacing = replaced.clone();
        let staged = destination.clone();
        let racer: super::fs_quirks::Racer = Box::new(move || {
            for entry in fs::read_dir(&staged).expect("list the destination") {
                let copy = entry.expect("an entry").path().join("notes.txt");
                if copy.exists() {
                    fs::write(&copy, b"another program's file").expect("replace the copy");
                    fs::set_permissions(&copy, fs::Permissions::from_mode(mode))
                        .expect("set the replacement's mode");
                    replacing.store(true, Ordering::Release);
                    return;
                }
            }
        });
        let _sources = FsQuirks::install(&sources, Quirks::ext4());
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                after: vec![(Op::Close, "notes.txt", 0, racer)],
                ..preset()
            },
        );

        let (_operation, events) =
            paste_move(98, &destination, &source, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        assert!(replaced.load(Ordering::Acquire), "{row}");
        assert_eq!(fs::read(&notes).ok(), Some(b"notes".to_vec()), "{row}");
        assert_eq!(
            reported(&events).kept,
            [kept(source, KeptCause::CopyGone)],
            "{row}"
        );
    }
    Ok(())
}

#[test]
fn a_special_file_move_keeps_its_source_without_proven_published_ownership()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (pipe, can_place, replace) in [
        (false, true, true),
        (true, true, true),
        (false, false, true),
        (true, false, true),
        (false, false, false),
        (true, false, false),
    ] {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let source = sources.join("special");
        if pipe {
            rustix::fs::mknodat(
                rustix::fs::CWD,
                &source,
                rustix::fs::FileType::Fifo,
                rustix::fs::Mode::from_raw_mode(0o600),
                0,
            )?;
        } else {
            std::os::unix::fs::symlink("original-target", &source)?;
        }
        let arrival = root.path().join("arrival");
        fs::write(&arrival, b"another program's file")?;
        fs::set_permissions(&arrival, fs::Permissions::from_mode(0o640))?;
        let replaced = Arc::new(AtomicBool::new(false));
        let creation = if pipe { Op::CreateNode } else { Op::Symlink };
        let after = std::iter::once(creation)
            .chain(can_place.then_some(Op::Rename))
            .filter(|_| replace)
            .map(|op| {
                let arrival = arrival.clone();
                let target = destination.join("special");
                let replaced = replaced.clone();
                let racer: super::fs_quirks::Racer = Box::new(move || {
                    if !replaced.swap(true, Ordering::AcqRel) {
                        fs::rename(&arrival, &target).expect("replace the published special file");
                    }
                });
                (op, "special", 0, racer)
            })
            .collect();
        let _sources = FsQuirks::install(&sources, Quirks::default());
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                after,
                ..if can_place {
                    Quirks::default()
                } else {
                    Quirks::without_placement()
                }
            },
        );
        let (_operation, events) =
            paste_move(99, &destination, &source, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        let row = format!("pipe: {pipe}, can_place: {can_place}, replace: {replace}");
        assert_eq!(replaced.load(Ordering::Acquire), replace, "{row}");
        // A link made again at its name holds the copy once it points where
        // the staged link did.
        let proven = !pipe && !replace;
        assert_eq!(
            fs::symlink_metadata(&source).is_ok(),
            !proven,
            "{row}: {:?}",
            events.borrow()
        );
        if replace {
            assert_eq!(
                fs::read(destination.join("special"))?,
                b"another program's file"
            );
            assert_eq!(
                fs::metadata(destination.join("special"))?.mode() & 0o777,
                0o640
            );
        } else {
            let expected = if proven {
                Vec::new()
            } else {
                vec![kept(source.clone(), KeptCause::CopyGone)]
            };
            assert_eq!(reported(&events).kept, expected, "{row}");
            if pipe {
                assert!(
                    fs::symlink_metadata(destination.join("special"))?
                        .file_type()
                        .is_fifo()
                );
            } else {
                assert_eq!(
                    fs::read_link(destination.join("special"))?,
                    PathBuf::from("original-target")
                );
            }
        }
        if pipe {
            assert!(fs::symlink_metadata(&source)?.file_type().is_fifo());
        } else if !proven {
            assert_eq!(fs::read_link(&source)?, PathBuf::from("original-target"));
        }
    }
    Ok(())
}

#[test]
fn copies_a_mount_numbers_afresh_are_known_by_birth_time_or_else_by_inode_number()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
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
    // exFAT and FUSE daemons without `use_ino` number an entry afresh once
    // the kernel evicts it, here right after the copy is published; on ext4,
    // which keeps inode numbers, another number is another file. Merging into
    // a folder on exFAT sets its birth time again. Whether the move deletes
    // its source.
    let (fail, merge) = (TransferConflict::FailIfExists, TransferConflict::Merge);
    let rows: [(Preset, &str, TransferConflict, Change, bool); 11] = [
        (Quirks::sshfs, "notes.txt", fail, Change::None, false),
        (Quirks::sshfs, "folder", fail, Change::FolderEvicted, false),
        (
            Quirks::exfat_fuse,
            "folder",
            fail,
            Change::FolderEvicted,
            false,
        ),
        (Quirks::vfat, "folder", fail, Change::None, true),
        (Quirks::exfat, "folder", fail, Change::None, true),
        (Quirks::exfat, "folder", fail, Change::Rewritten, false),
        (
            Quirks::exfat_fuse,
            "folder",
            fail,
            Change::ReplacedWithinTheTick,
            false,
        ),
        (Quirks::ext4, "notes.txt", fail, Change::None, false),
        (Quirks::exfat, "folder", merge, Change::None, true),
        (Quirks::exfat, "folder", merge, Change::ReplacedAlike, false),
        (Quirks::vfat, "folder", merge, Change::FolderReplaced, false),
    ];
    for (preset, item, conflict, change, deleted) in rows {
        let row = format!(
            "{:?}, {item}, {conflict:?}, {change:?}",
            preset().mount_type
        );
        let within_the_tick = change == Change::ReplacedWithinTheTick;
        // Before the kernel's coarse clock, which may run a tick behind.
        let tick = SystemTime::now() - Duration::from_secs(1);
        let _clock = within_the_tick.then(|| ClockGuard::set(tick));
        // Multigrain source times: only the destination can call for a
        // comparison of contents.
        let _kernel = KernelGuard::set(Some((6, 13, 0)));
        let coarse_clock = within_the_tick.then_some(tick);
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let source = sources.join(item);
        let copy = destination.join(item);
        let mut copied = vec![copy.clone()];
        let notes = if item == "folder" {
            fs::create_dir_all(source.join("sub"))?;
            fs::write(source.join("sub/b.txt"), b"b")?;
            copied.extend(["notes.txt", "sub", "sub/b.txt"].map(|path| copy.join(path)));
            source.join("notes.txt")
        } else {
            source.clone()
        };
        if conflict == merge {
            fs::create_dir(&copy)?;
            fs::write(copy.join("stays.txt"), b"stays")?;
        }
        fs::write(&notes, b"notes")?;
        let arrival = root.path().join("arrival");
        fs::write(&arrival, b"other")?;
        if matches!(change, Change::Rewritten | Change::ReplacedAlike) {
            let hour_before = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
            for file in [&notes, &arrival] {
                set_times(file, hour_before.as_secs() - 3600, 0)?;
            }
        }
        // What replaces the copy must be born in a later tick than what it stands in for.
        let probe = root.path().join("probe");
        match change {
            Change::ReplacedAlike => wait_for_a_later_birth_time(&probe, &fs::metadata(&arrival)?),
            Change::FolderReplaced => wait_for_a_later_birth_time(&probe, &fs::metadata(&copy)?),
            _ => {}
        }
        let copied_notes = if item == "folder" {
            copy.join("notes.txt")
        } else {
            copy.clone()
        };
        let replaced = copied_notes.clone();
        let merged = copy.clone();
        if change == Change::FolderEvicted {
            copied.truncate(1);
        }
        let evicted = Arc::new(AtomicBool::new(false));
        let evicting = evicted.clone();
        let racer: super::fs_quirks::Racer = Box::new(move || {
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
                super::fs_quirks::renumbered(path);
            }
            evicting.store(true, Ordering::Release);
        });
        let placed_last = if conflict == merge { "sub" } else { item };
        let _sources = FsQuirks::install(
            &sources,
            Quirks {
                coarse_clock,
                ..Quirks::ext4()
            },
        );
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                rename_flags: Support::Supported,
                tmpfile: Support::Rejected(Errno::OPNOTSUPP),
                coarse_clock,
                after: vec![(Op::Rename, placed_last, 0, racer)],
                ..preset()
            },
        );

        let (_operation, events) = paste_move(98, &destination, &source, conflict);
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{row}: {:?}",
            events.borrow()
        );
        assert!(evicted.load(Ordering::Acquire), "{row}");
        if deleted {
            assert!(!source.exists(), "{row}: {:?}", reported(&events).kept);
            assert_eq!(fs::read(&copied_notes)?, b"notes", "{row}");
        } else {
            assert_eq!(fs::read(&notes).ok(), Some(b"notes".to_vec()), "{row}");
            assert_eq!(
                reported(&events).kept,
                [kept(source, KeptCause::CopyGone)],
                "{row}"
            );
        }
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
        let made_again = matches!(preset().rename_flags, Support::Rejected(_));
        let row = format!(
            "{:?}, {item}, {conflict:?}, made again: {made_again}, replaced: {replaced}",
            preset().mount_type
        );
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let source = sources.join(item);
        let copy = destination.join(item);
        let (link, copied) = if item == "folder" {
            fs::create_dir(&source)?;
            (source.join("link"), copy.join("link"))
        } else {
            (source.clone(), copy.clone())
        };
        let placed = if conflict == merge {
            fs::create_dir(&copy)?;
            "link"
        } else {
            item
        };
        std::os::unix::fs::symlink("original-target", &link)?;
        let arrival = root.path().join("arrival");
        std::os::unix::fs::symlink("another-target", &arrival)?;
        let evicted = Arc::new(AtomicBool::new(false));
        let evicting = evicted.clone();
        let replacing = copied.clone();
        let racer: super::fs_quirks::Racer = Box::new(move || {
            if replaced {
                fs::rename(&arrival, &replacing).expect("replace the link");
            }
            super::fs_quirks::renumbered(&replacing);
            evicting.store(true, Ordering::Release);
        });
        let _sources = FsQuirks::install(&sources, Quirks::ext4());
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                tmpfile: Support::Rejected(Errno::OPNOTSUPP),
                // A link in a staged folder is first made there under its name.
                after: vec![if made_again {
                    (Op::Symlink, placed, usize::from(item == "folder"), racer)
                } else {
                    (Op::Rename, placed, 0, racer)
                }],
                ..preset()
            },
        );

        let (_operation, events) = paste_move(98, &destination, &source, conflict);
        pump_until_transfer(&events);

        assert!(evicted.load(Ordering::Acquire), "{row}");
        let original = Some(PathBuf::from("original-target"));
        if replaced {
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
    }
    Ok(())
}

#[test]
fn a_replacing_move_keeps_the_actual_copy_out_identity() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    let source = sources.join("folder");
    fs::create_dir_all(&source)?;
    fs::create_dir_all(destination.join("folder"))?;
    fs::write(source.join("notes.txt"), b"notes")?;
    fs::write(destination.join("folder/old.txt"), b"old")?;
    let _sources = FsQuirks::install(&sources, Quirks::default());
    let destination_scope = FsQuirks::install(
        &destination,
        Quirks {
            exchange: super::fs_quirks::Exchange::Rejected,
            ..Quirks::vboxsf()
        },
    );
    let (_operation, events) =
        paste_move(98, &destination, &source, TransferConflict::ReplaceExisting);
    pump_until_transfer(&events);
    assert!(
        matches!(
            terminal_transfer(&events.borrow()),
            Some(OperationEvent::Pasted { .. })
        ),
        "{:?}",
        events.borrow()
    );
    assert_eq!(fs::read(destination.join("folder/notes.txt"))?, b"notes");
    assert!(
        destination_scope
            .calls()
            .iter()
            .any(|call| call.op == Op::Mkdir && call.names.iter().any(|name| name == "folder")),
        "exclusive copy-out created the final folder"
    );
    assert!(!source.exists());
    assert!(reported(&events).kept.is_empty(), "{:?}", reported(&events));
    Ok(())
}

#[test]
fn later_names_rewritten_within_the_tick_are_kept() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let _kernel = KernelGuard::set(Some((6, 12, 0)));
    // Within one coarse tick only a digest tells a same-size rewrite. SMB
    // refuses to open a later name, so none of them can be compared there;
    // which of the unchanged pair is later depends on the listing order.
    for (row, quirks, later_kept) in [
        ("ext4", Quirks::ext4 as fn() -> Quirks, false),
        ("smb", smb_with_one_name_open, true),
    ] {
        let tick = SystemTime::now() - Duration::from_secs(1);
        let _clock = ClockGuard::set(tick);
        let root = tempfile::tempdir()?;
        let folder = root.path().join("source/folder");
        fs::create_dir_all(folder.join("sub"))?;
        for (first, second) in [("x", "sub/y"), ("m", "sub/n")] {
            fs::write(folder.join(first), first)?;
            fs::hard_link(folder.join(first), folder.join(second))?;
        }
        let rewritten = folder.join("m");
        let racer: super::fs_quirks::Racer =
            Box::new(move || fs::write(&rewritten, b"M").expect("rewrite m"));
        let quirks = Quirks {
            before: vec![(Op::OpenDir, "folder", 1, racer)],
            coarse_clock: Some(tick),
            ..quirks()
        };

        let (moved, report, _) = move_across_volumes(&folder, quirks)?;

        assert_eq!(fs::read(moved.join("sub/n"))?, b"m", "{row}");
        let mut left = report.kept;
        left.sort_by_key(|item| item.location.native_path().map(Path::to_path_buf));
        let unchanged_pair = [folder.join("x"), folder.join("sub/y")];
        let later = unchanged_pair
            .iter()
            .filter(|name| {
                left.iter()
                    .any(|item| item.location == Location::local(name))
            })
            .collect::<Vec<_>>();
        assert_eq!(later.len(), usize::from(later_kept), "{row}: {left:?}");
        let mut expected = vec![
            kept(folder.join("m"), KeptCause::Changed),
            kept(folder.join("sub/n"), KeptCause::Changed),
        ];
        expected.extend(
            later
                .iter()
                .map(|name| kept((*name).clone(), KeptCause::Changed)),
        );
        expected.sort_by_key(|item| item.location.native_path().map(Path::to_path_buf));
        assert_eq!(left, expected, "{row}");
        for name in &unchanged_pair {
            assert_eq!(name.exists(), later.contains(&name), "{row}: {name:?}");
        }
    }
    Ok(())
}

fn smb_with_one_name_open() -> Quirks {
    Quirks {
        mount_type: Some(("cifs", "rw")),
        name_opens: NameOpens::OneAtATime,
        ..Quirks::default()
    }
}

#[test]
fn linked_names_on_an_smb_source_are_never_open_at_once() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("source/folder");
    fs::create_dir_all(folder.join("sub"))?;
    fs::write(folder.join("a"), b"linked")?;
    let hour_before = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs()
        - 3600;
    set_times(&folder.join("a"), hour_before, 0)?;
    fs::hard_link(folder.join("a"), folder.join("sub/b"))?;
    fs::hard_link(folder.join("a"), folder.join("sub/c"))?;
    let smb = Quirks {
        mount_type: Some(("cifs", "rw")),
        name_opens: NameOpens::FirstOnly,
        ..Quirks::default()
    };

    let (moved, report, _) = move_across_volumes(&folder, smb)?;

    assert!(fs::symlink_metadata(&folder).is_err());
    assert_eq!(report, TransferReport::default());
    let [a, b, c] = ["a", "sub/b", "sub/c"].map(|name| fs::metadata(moved.join(name)));
    let (a, b, c) = (a?, b?, c?);
    assert_eq!((a.ino(), a.nlink()), (b.ino(), 3));
    assert_eq!(c.ino(), a.ino());
    assert_eq!(fs::read(moved.join("sub/c"))?, b"linked");
    Ok(())
}

#[test]
fn a_file_put_in_a_moved_files_place_within_its_time_tick_is_kept() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("source/folder");
    fs::create_dir_all(&folder)?;
    let moved_file = folder.join("a.txt");
    fs::write(&moved_file, b"original")?;
    let substituted = moved_file.clone();
    let racer: super::fs_quirks::Racer = Box::new(move || {
        let before = fs::metadata(&substituted).expect("stat the original");
        let other = substituted.with_file_name("other");
        fs::write(&other, b"newfile!").expect("write the other file");
        let modified = rustix::fs::Timespec {
            tv_sec: before.mtime(),
            tv_nsec: before.mtime_nsec(),
        };
        rustix::fs::utimensat(
            rustix::fs::CWD,
            &other,
            &rustix::fs::Timestamps {
                last_access: modified,
                last_modification: modified,
            },
            rustix::fs::AtFlags::empty(),
        )
        .expect("give it the same times");
        fs::rename(&other, &substituted).expect("put it in the original's place");
        super::fs_quirks::recreated(&substituted);
    });
    let fat = Quirks {
        mount_type: Some(("vfat", "rw")),
        ctime_is_mtime: true,
        reused_inodes: Some("a.txt"),
        before: vec![(Op::OpenDir, "folder", 1, racer)],
        ..Quirks::default()
    };

    let (moved, report, _) = move_across_volumes(&folder, fat)?;

    assert_eq!(fs::read(moved.join("a.txt"))?, b"original");
    assert_eq!(fs::read(&moved_file)?, b"newfile!");
    assert_eq!(report.kept, [kept(moved_file, KeptCause::Changed)]);
    Ok(())
}

#[test]
fn files_read_within_a_coarse_time_tick_are_compared_before_their_move_deletes_them()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // FAT, exFAT, sshfs and HFS+ keep coarse times, so a rewrite of the
    // same size within one tick shows neither in the modification time nor
    // in the change time, which they lack or keep as coarse.
    let vfat = || Quirks {
        ctime_is_mtime: true,
        ..Quirks::vfat()
    };
    fn exfat_fuse() -> Quirks {
        Quirks {
            ctime_is_mtime: true,
            ..Quirks::exfat_fuse()
        }
    }
    // exfat-fuse on a device udev has no record of, or cannot read.
    let fuseblk = || Quirks {
        device_format: None,
        ..exfat_fuse()
    };
    let udev_failed = || Quirks {
        fail: vec![(Op::DeviceFormat, "*", 0, Errno::ACCESS)],
        ..exfat_fuse()
    };
    let sshfs = || Quirks {
        ctime_is_mtime: true,
        ..Quirks::sshfs()
    };
    let hfsplus = || Quirks {
        mount_type: Some(("hfsplus", "rw")),
        ctime_is_mtime: true,
        ..Quirks::default()
    };
    // An SMB share of a FAT disk, whose times only their lack of a fraction
    // of a second tells coarse.
    let fat_share = || Quirks {
        ctime_is_mtime: true,
        ..Quirks::cifs()
    };
    // Reads of four bytes, so the copy and the comparison read in chunks.
    let chunked = || Quirks {
        short_io: Some(4),
        ctime_is_mtime: true,
        ..Quirks::vfat()
    };
    let recent = 500;
    let past_the_tick = 2050;
    let behind = 2500;
    let long_ago = 3_600_000;
    let ahead = -3_600_000;
    #[derive(Clone, Copy, PartialEq)]
    enum Rewrite {
        Never,
        Deleting,
        Comparing,
        Compared,
        Grown,
        RenamedOver,
        Reached,
        ReachedCompared,
        ReachedAtStat,
        ChunkCompared,
        ClockSetBack,
    }
    // How many milliseconds after its modification time the copy reads the file
    // (negative: dated ahead of the clock), and whether the deletion reads it
    // again.
    let rows: [(&str, Preset, i64, Rewrite, bool); 24] = [
        ("vfat", vfat, recent, Rewrite::Deleting, true),
        (
            "vfat, past the tick",
            vfat,
            past_the_tick,
            Rewrite::Deleting,
            true,
        ),
        ("exfat-fuse", exfat_fuse, recent, Rewrite::Deleting, true),
        (
            "fuseblk, no record",
            fuseblk,
            recent,
            Rewrite::Deleting,
            true,
        ),
        (
            "fuseblk, udev failed",
            udev_failed,
            recent,
            Rewrite::Deleting,
            true,
        ),
        (
            "exfat-fuse, renamed over",
            exfat_fuse,
            recent,
            Rewrite::RenamedOver,
            true,
        ),
        ("sshfs", sshfs, recent, Rewrite::Deleting, true),
        ("sshfs, behind", sshfs, behind, Rewrite::Deleting, true),
        ("hfsplus", hfsplus, recent, Rewrite::Deleting, true),
        ("FAT share", fat_share, recent, Rewrite::Deleting, true),
        ("vfat, file", vfat, recent, Rewrite::Comparing, true),
        ("vfat, compared", vfat, recent, Rewrite::Compared, true),
        ("vfat, grown", vfat, recent, Rewrite::Grown, true),
        ("vfat, unchanged", vfat, recent, Rewrite::Never, true),
        ("vfat, in chunks", chunked, recent, Rewrite::Never, true),
        (
            "vfat, chunk compared",
            chunked,
            recent,
            Rewrite::ChunkCompared,
            true,
        ),
        (
            "vfat, clock set back",
            vfat,
            recent,
            Rewrite::ClockSetBack,
            false,
        ),
        ("vfat, long before", vfat, long_ago, Rewrite::Never, false),
        ("vfat, ahead", vfat, ahead, Rewrite::Never, false),
        ("vfat, ahead, reached", vfat, ahead, Rewrite::Reached, true),
        (
            "vfat, ahead, compared",
            vfat,
            ahead,
            Rewrite::ReachedCompared,
            true,
        ),
        (
            "vfat, ahead, at the stat",
            vfat,
            ahead,
            Rewrite::ReachedAtStat,
            false,
        ),
        ("sshfs, ahead", sshfs, ahead, Rewrite::Never, true),
        ("ext4", Quirks::ext4, recent, Rewrite::Never, false),
    ];
    for (row, preset, read_after, rewrite, read_again) in rows {
        let root = tempfile::tempdir()?;
        let folder = root.path().join("source/folder");
        fs::create_dir_all(&folder)?;
        let file = folder.join("a.txt");
        fs::write(&file, b"original")?;
        let epoch = SystemTime::UNIX_EPOCH;
        let seconds = SystemTime::now().duration_since(epoch)?.as_secs() / 2 * 2;
        // ext4 keeps nanoseconds and exFAT hundredths of a second; FAT,
        // sshfs and HFS+ keep whole seconds.
        let fraction = match preset().mount_type {
            Some(("ext4", _)) => 250_000_000,
            Some(("fuseblk", _)) => 500_000_000,
            _ => 0,
        };
        set_times(&file, seconds, fraction)?;
        let read_at = i64::try_from(seconds)? * 1000 + read_after;
        let _clock = ClockGuard::set(epoch + Duration::from_millis(u64::try_from(read_at)?));
        // A rewrite keeps the file's times, or, `clocked`, takes the kernel's
        // coarse clock, a tick behind the engine's, in FAT's two-second steps.
        let rewriter = |contents: &'static [u8], clocked: bool| -> super::fs_quirks::Racer {
            let path = file.clone();
            Box::new(move || {
                fs::write(&path, contents).expect("rewrite the source");
                let now = (super::super::copy_engine::clock_for_test() - Duration::from_millis(10))
                    .duration_since(epoch)
                    .expect("a time after 1970");
                let (step, fraction) = if clocked {
                    (now.as_secs() / 2 * 2, 0)
                } else {
                    (seconds, fraction)
                };
                set_times(&path, step, fraction).expect("set the times");
            })
        };
        let reached = |mut then: super::fs_quirks::Racer| -> super::fs_quirks::Racer {
            Box::new(move || {
                let after = epoch + Duration::from_secs(seconds) + Duration::from_millis(500);
                super::super::copy_engine::set_clock_for_test(Some(after));
                then();
            })
        };
        let (item, before, after) = match rewrite {
            Rewrite::Never => (&folder, Vec::new(), Vec::new()),
            Rewrite::Deleting => (
                &folder,
                vec![(Op::OpenDir, "folder", 1, rewriter(b"ORIGINAL", false))],
                Vec::new(),
            ),
            Rewrite::Comparing => (
                &file,
                vec![(Op::OpenSource, "a.txt", 1, rewriter(b"ORIGINAL", false))],
                Vec::new(),
            ),
            Rewrite::Compared => (
                &folder,
                vec![(Op::Stat, "a.txt", 0, rewriter(b"ORIGINAL", true))],
                Vec::new(),
            ),
            Rewrite::Grown => (
                &folder,
                vec![
                    (Op::Read, "a.txt", 0, rewriter(b"original, grown", false)),
                    (
                        Op::OpenDir,
                        "folder",
                        1,
                        rewriter(b"ORIGINAL, GROWN", false),
                    ),
                ],
                Vec::new(),
            ),
            Rewrite::RenamedOver => {
                let (path, saved) = (file.clone(), folder.join(".a.txt.save"));
                let rename_over: super::fs_quirks::Racer = Box::new(move || {
                    fs::write(&saved, b"ORIGINAL").expect("save the new version aside");
                    set_times(&saved, seconds, fraction).expect("set the times");
                    fs::rename(&saved, &path).expect("rename it over the source");
                });
                (
                    &folder,
                    Vec::new(),
                    vec![(Op::OpenSource, "a.txt", 1, rename_over)],
                )
            }
            Rewrite::Reached => (
                &folder,
                vec![(
                    Op::OpenDir,
                    "folder",
                    1,
                    reached(rewriter(b"ORIGINAL", false)),
                )],
                Vec::new(),
            ),
            Rewrite::ReachedCompared => (
                &folder,
                vec![
                    (Op::OpenDir, "folder", 1, reached(Box::new(|| {}))),
                    (Op::Stat, "a.txt", 0, rewriter(b"ORIGINAL", true)),
                ],
                Vec::new(),
            ),
            Rewrite::ReachedAtStat => (
                &folder,
                vec![(Op::Stat, "a.txt", 0, reached(rewriter(b"ORIGINAL", false)))],
                Vec::new(),
            ),
            Rewrite::ChunkCompared => {
                let path = file.clone();
                let rewrite_chunk: super::fs_quirks::Racer = Box::new(move || {
                    let source = fs::OpenOptions::new().write(true).open(&path);
                    std::os::unix::fs::FileExt::write_at(&source.expect("open"), b"ORIG", 0)
                        .expect("rewrite the first chunk");
                    let now = (super::super::copy_engine::clock_for_test()
                        - Duration::from_millis(10))
                    .duration_since(epoch)
                    .expect("a time after 1970");
                    set_times(&path, now.as_secs() / 2 * 2, 0).expect("set the times");
                });
                // The copy reads two chunks and the end, then the comparison
                // its first chunk.
                (
                    &folder,
                    Vec::new(),
                    vec![(Op::Read, "a.txt", 3, rewrite_chunk)],
                )
            }
            Rewrite::ClockSetBack => {
                let set_back: super::fs_quirks::Racer = Box::new(move || {
                    let hour_before = epoch + Duration::from_secs(seconds - 3600);
                    super::super::copy_engine::set_clock_for_test(Some(hour_before));
                });
                (
                    &folder,
                    vec![(Op::OpenDir, "folder", 1, set_back)],
                    Vec::new(),
                )
            }
        };
        let quirks = Quirks {
            before,
            after,
            ..preset()
        };

        let (moved, report, _, calls) = move_across_volumes_logged(item, quirks)?;

        let copied = if item == &file {
            moved
        } else {
            moved.join("a.txt")
        };
        let grown = rewrite == Rewrite::Grown;
        let original: &[u8] = if grown {
            b"original, grown"
        } else {
            b"original"
        };
        assert_eq!(fs::read(copied)?, original, "{row}");
        match rewrite {
            Rewrite::Never => {
                assert!(!item.exists(), "{row}");
                assert_eq!(report.kept, [], "{row}");
            }
            Rewrite::ClockSetBack => {
                assert_eq!(fs::read(&file)?, original, "{row}");
                assert_eq!(report.kept, [kept(file, KeptCause::Changed)], "{row}");
            }
            Rewrite::ChunkCompared => {
                assert_eq!(fs::read(&file)?, b"ORIGinal", "{row}");
                assert_eq!(report.kept, [kept(file, KeptCause::Changed)], "{row}");
            }
            _ => {
                assert_eq!(fs::read(&file)?, original.to_ascii_uppercase(), "{row}");
                assert_eq!(report.kept, [kept(file, KeptCause::Changed)], "{row}");
            }
        }
        let opened: Vec<usize> = calls
            .iter()
            .enumerate()
            .filter(|(_, call)| call.op == Op::OpenSource && call.names == ["a.txt"])
            .map(|(index, _)| index)
            .collect();
        // A file renamed over reads as deleted through the descriptor.
        let read = opened.get(1).is_some_and(|&reopened| {
            calls[reopened..].iter().any(|call| {
                call.op == Op::Read
                    && (call.names == ["a.txt"] || call.names == ["a.txt (deleted)"])
            })
        });
        assert_eq!(read, read_again, "{row}");
    }
    Ok(())
}

#[test]
fn files_on_a_coarse_mount_inside_a_moved_folder_are_compared() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // Multigrain times on the outer mount, so outer.txt, dated around the
    // frozen clock, is never compared and waited for.
    let _kernel = KernelGuard::set(Some((6, 13, 0)));
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let folder = source_dir.join("folder");
    let inner = folder.join("inner");
    fs::create_dir_all(&inner)?;
    fs::write(folder.join("outer.txt"), b"outer")?;
    let file = inner.join("a.txt");
    fs::write(&file, b"original")?;
    let epoch = SystemTime::UNIX_EPOCH;
    let seconds = SystemTime::now().duration_since(epoch)?.as_secs() / 2 * 2;
    // exFAT keeps hundredths of a second.
    let fraction = 500_000_000;
    set_times(&file, seconds, fraction)?;
    let _clock =
        ClockGuard::set(epoch + Duration::new(seconds, fraction) + Duration::from_millis(500));
    let path = file.clone();
    let rewrite: super::fs_quirks::Racer = Box::new(move || {
        fs::write(&path, b"ORIGINAL").expect("rewrite the source");
        set_times(&path, seconds, fraction).expect("keep its times");
    });
    let outer = FsQuirks::install(
        &source_dir,
        Quirks {
            before: vec![(Op::OpenDir, "folder", 1, rewrite)],
            ..Quirks::ext4()
        },
    );
    // The outer file is read first, as a single worker does anyway.
    let calls = outer.reader();
    let outer_read: super::fs_quirks::Racer = Box::new(move || {
        let start = Instant::now();
        while !calls()
            .iter()
            .any(|call| call.op == Op::Read && call.names == ["outer.txt"])
        {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "outer.txt was never read"
            );
            thread::sleep(Duration::from_millis(5));
        }
    });
    let _inner = FsQuirks::install(
        &inner,
        Quirks {
            mount_type: Some(("fuseblk", "rw")),
            device_format: Some("exfat"),
            ctime_is_mtime: true,
            no_birth_time: true,
            before: vec![(Op::OpenSource, "a.txt", 0, outer_read)],
            ..Quirks::default()
        },
    );
    let destination = root.path().join("destination");
    fs::create_dir_all(&destination)?;
    let _destination = FsQuirks::install(&destination, Quirks::default());

    let (_operation, events) =
        paste_move(97, &destination, &folder, TransferConflict::FailIfExists);
    pump_until_transfer(&events);

    let report = reported(&events);
    assert_eq!(
        fs::read(destination.join("folder/inner/a.txt"))?,
        b"original"
    );
    assert_eq!(fs::read(&file)?, b"ORIGINAL");
    assert_eq!(report.kept, [kept(file, KeptCause::Changed)]);
    Ok(())
}

#[test]
fn files_modified_within_a_tick_of_the_clock_stamping_them_are_compared_before_deletion()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let ramfs = || Quirks {
        mount_type: Some(("ramfs", "rw")),
        ..Quirks::default()
    };
    // Since Linux 6.13, ext4, XFS, btrfs and tmpfs stamp a change right after
    // a stat with a finer time; elsewhere a same-size rewrite within the tick
    // keeps the times. How many milliseconds after the tick the copy reads the
    // file, whether it is rewritten as the deletion starts, and whether the
    // deletion reads it again.
    type Row = (&'static str, (u32, u32, u32), Preset, u64, bool, bool);
    let rows: [Row; 6] = [
        ("ext4 before 6.13", (6, 12, 0), Quirks::ext4, 5, true, true),
        (
            "ext4 before 6.13, untouched",
            (6, 12, 0),
            Quirks::ext4,
            5,
            false,
            true,
        ),
        (
            "ext4 before 6.13, a tick later",
            (6, 12, 0),
            Quirks::ext4,
            150,
            false,
            false,
        ),
        ("ext4 since 6.13", (6, 13, 0), Quirks::ext4, 5, false, false),
        ("ramfs since 6.13", (6, 13, 0), ramfs, 5, true, true),
        ("NFS since 6.13", (6, 13, 0), Quirks::nfs42, 5, true, true),
    ];
    for (row, kernel, preset, read_after, rewritten, read_again) in rows {
        let _kernel = KernelGuard::set(Some(kernel));
        // Before the kernel's coarse clock, which may run a tick behind.
        let tick = SystemTime::now() - Duration::from_secs(1);
        let _clock = ClockGuard::set(tick + Duration::from_millis(read_after));
        let root = tempfile::tempdir()?;
        let folder = root.path().join("source/folder");
        fs::create_dir_all(&folder)?;
        let file = folder.join("a.txt");
        fs::write(&file, b"original")?;
        let before = if rewritten {
            vec![(Op::OpenDir, "folder", 1, rewrite(&file, b"ORIGINAL"))]
        } else {
            Vec::new()
        };
        let quirks = Quirks {
            before,
            coarse_clock: Some(tick),
            ..preset()
        };

        let (moved, report, _, calls) = move_across_volumes_logged(&folder, quirks)?;

        assert_eq!(fs::read(moved.join("a.txt"))?, b"original", "{row}");
        if rewritten {
            assert_eq!(fs::read(&file).ok(), Some(b"ORIGINAL".to_vec()), "{row}");
            assert_eq!(report.kept, [kept(file, KeptCause::Changed)], "{row}");
        } else {
            assert!(!folder.exists(), "{row}");
            assert_eq!(report.kept, [], "{row}");
        }
        let opened = calls
            .iter()
            .filter(|call| call.op == Op::OpenSource && call.names == ["a.txt"])
            .count();
        assert_eq!(opened > 1, read_again, "{row}");
    }
    Ok(())
}

#[test]
fn a_source_growing_as_it_is_compared_is_kept_without_reading_what_it_grew_by()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("source/folder");
    fs::create_dir_all(&folder)?;
    let file = folder.join("a.txt");
    fs::write(&file, b"original")?;
    let epoch = SystemTime::UNIX_EPOCH;
    let seconds = SystemTime::now().duration_since(epoch)?.as_secs() / 2 * 2;
    set_times(&file, seconds, 0)?;
    let _clock = ClockGuard::set(epoch + Duration::from_secs(seconds) + Duration::from_millis(500));
    let grown = 64 << 20;
    let path = file.clone();
    let grow: super::fs_quirks::Racer = Box::new(move || {
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|file| file.set_len(grown))
            .expect("grow the source");
    });
    let quirks = Quirks {
        ctime_is_mtime: true,
        after: vec![(Op::OpenSource, "a.txt", 1, grow)],
        ..Quirks::vfat()
    };

    let (moved, report, _, calls) = move_across_volumes_logged(&folder, quirks)?;

    assert_eq!(fs::read(moved.join("a.txt"))?, b"original");
    assert_eq!(fs::metadata(&file)?.len(), grown);
    assert_eq!(report.kept, [kept(file, KeptCause::Changed)]);
    let reopened = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| call.op == Op::OpenSource && call.names == ["a.txt"])
        .nth(1)
        .map(|(index, _)| index)
        .ok_or("the deletion opened the source to compare it")?;
    let reads = calls[reopened..]
        .iter()
        .filter(|call| call.op == Op::Read && call.names == ["a.txt"])
        .count();
    assert!(reads <= 2, "{reads} reads of a source that grew by 64 MiB");
    Ok(())
}
