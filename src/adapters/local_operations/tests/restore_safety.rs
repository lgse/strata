// SPDX-License-Identifier: MIT

use super::super::{
    copy_restore_entry, discard_restore_copy, move_restore_path, move_restore_path_with,
};
use super::*;
use rustix::{fs::RenameFlags, io::Errno};
use std::os::unix::fs::symlink;

#[test]
fn unsupported_restore_rename_never_downgrades_atomicity() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    for error in [Errno::INVAL, Errno::NOSYS, Errno::OPNOTSUPP] {
        for directory in [false, true] {
            let fixture = tempfile::tempdir()?;
            let source = fixture.path().join("source");
            let destination = fixture.path().join("destination");
            if directory {
                fs::create_dir(&source)?;
                fs::write(source.join("contents"), b"original")?;
            } else {
                fs::write(&source, b"original")?;
            }
            let raced = destination.clone();
            let result = glib::MainContext::default().block_on(move_restore_path_with(
                source.clone(),
                destination.clone(),
                fixture.path().to_path_buf(),
                gio::Cancellable::new(),
                move |_, _, _, _, flags| {
                    assert_eq!(flags, RenameFlags::NOREPLACE);
                    fs::write(&raced, b"concurrent user data").expect("racing destination");
                    Err(error)
                },
            ));
            let message = result
                .expect_err("unsupported atomic operation")
                .to_string();
            assert!(
                message.contains("something already exists at the destination"),
                "{message}"
            );
            assert!(!message.contains("across volumes"));
            assert_eq!(fs::read(&destination)?, b"concurrent user data");
            assert_eq!(
                fs::read(if directory {
                    source.join("contents")
                } else {
                    source
                })?,
                b"original"
            );
        }
    }
    Ok(())
}

#[test]
fn unsupported_restore_rename_safely_restores_when_destination_available()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    for error in [Errno::INVAL, Errno::NOSYS, Errno::OPNOTSUPP] {
        for directory in [false, true] {
            let fixture = tempfile::tempdir()?;
            let source = fixture.path().join("source");
            let destination = fixture.path().join("destination");
            if directory {
                fs::create_dir(&source)?;
                fs::write(source.join("contents"), b"original")?;
            } else {
                fs::write(&source, b"original")?;
            }
            let result = glib::MainContext::default().block_on(move_restore_path_with(
                source.clone(),
                destination.clone(),
                fixture.path().to_path_buf(),
                gio::Cancellable::new(),
                move |_, _, _, _, flags| {
                    assert_eq!(flags, RenameFlags::NOREPLACE);
                    Err(error)
                },
            ));
            assert!(result.is_ok(), "restore should succeed: {result:?}");
            assert!(!source.exists(), "source should be removed from trash");
            if directory {
                assert_eq!(fs::read(destination.join("contents"))?, b"original");
            } else {
                assert_eq!(fs::read(&destination)?, b"original");
            }
        }
    }
    Ok(())
}

#[test]
fn unsupported_restore_keeps_both_parents_pinned() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let allowed = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let source_parent = allowed.path().join("trash");
    let target_parent = allowed.path().join("documents");
    fs::create_dir(&source_parent)?;
    fs::create_dir(&target_parent)?;
    let source = source_parent.join("report");
    fs::create_dir(&source)?;
    fs::write(source.join("contents"), b"original")?;
    fs::create_dir(outside.path().join("report"))?;
    fs::write(outside.path().join("report/contents"), b"unrelated")?;
    let retained_source = allowed.path().join("old-trash");
    let retained_target = allowed.path().join("old-documents");
    let old_source = retained_source.clone();
    let old_target = retained_target.clone();
    let escaped = outside.path().to_path_buf();
    glib::MainContext::default().block_on(move_restore_path_with(
        source,
        target_parent.join("report"),
        allowed.path().to_path_buf(),
        gio::Cancellable::new(),
        move |_, _, _, _, _| {
            fs::rename(&source_parent, old_source).expect("move source parent");
            fs::rename(&target_parent, old_target).expect("move destination parent");
            symlink(&escaped, &source_parent).expect("redirect source parent");
            symlink(&escaped, &target_parent).expect("redirect destination parent");
            Err(Errno::INVAL)
        },
    ))?;
    assert_eq!(
        fs::read(retained_target.join("report/contents"))?,
        b"original"
    );
    assert!(!retained_source.join("report").exists());
    assert_eq!(
        fs::read(outside.path().join("report/contents"))?,
        b"unrelated"
    );
    Ok(())
}

#[test]
fn unsupported_restore_preserves_a_replaced_source() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let source = fixture.path().join("source");
    let original = fixture.path().join("original");
    let destination = fixture.path().join("destination");
    fs::write(&source, b"original")?;
    let raced_source = source.clone();
    let retained = original.clone();
    let result = glib::MainContext::default().block_on(move_restore_path_with(
        source.clone(),
        destination.clone(),
        fixture.path().to_path_buf(),
        gio::Cancellable::new(),
        move |_, _, _, _, _| {
            fs::rename(&raced_source, retained).expect("retain original source");
            fs::write(&raced_source, b"replacement").expect("replace source");
            Err(Errno::INVAL)
        },
    ));
    assert!(result.is_err());
    assert_eq!(fs::read(source)?, b"replacement");
    assert_eq!(fs::read(original)?, b"original");
    assert!(!destination.exists());
    Ok(())
}

#[test]
fn restore_exclusive_copy_handles_files_links_and_partial_failure() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let source = fixture.path().join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("contents"), b"original")?;
    symlink("missing", source.join("link"))?;
    let parent = std::sync::Arc::new(super::super::open_local_parent_directory(fixture.path())?);
    glib::MainContext::default().block_on(copy_restore_entry(
        parent.clone(),
        "source".into(),
        parent.clone(),
        "destination".into(),
        None,
        gio::Cancellable::new(),
        Rc::new(RefCell::new(Vec::new())),
    ))?;
    assert_eq!(
        fs::read(fixture.path().join("destination/contents"))?,
        b"original"
    );
    assert_eq!(
        fs::read_link(fixture.path().join("destination/link"))?,
        Path::new("missing")
    );
    fs::write(fixture.path().join("occupied"), b"unrelated")?;
    assert!(
        glib::MainContext::default()
            .block_on(copy_restore_entry(
                parent.clone(),
                "source".into(),
                parent,
                "occupied".into(),
                None,
                gio::Cancellable::new(),
                Rc::new(RefCell::new(Vec::new())),
            ))
            .is_err()
    );
    assert_eq!(fs::read(fixture.path().join("occupied"))?, b"unrelated");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        source.join("fifo"),
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
        0,
    )?;
    let result = glib::MainContext::default().block_on(move_restore_path_with(
        source.clone(),
        fixture.path().join("incomplete"),
        fixture.path().to_path_buf(),
        gio::Cancellable::new(),
        |_, _, _, _, _| Err(Errno::INVAL),
    ));
    let message = result.expect_err("unsupported copy source").to_string();
    assert!(
        message.contains("incomplete copy could not be removed"),
        "{message}"
    );
    assert!(fixture.path().join("incomplete").is_dir());
    assert!(!fixture.path().join("incomplete/contents").exists());
    assert_eq!(fs::read(source.join("contents"))?, b"original");
    assert!(fs::symlink_metadata(source.join("fifo")).is_ok());
    Ok(())
}

#[test]
fn restore_copy_rechecks_the_opened_source_before_creating_output() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let source = fixture.path().join("source");
    fs::write(&source, b"original")?;
    let identity = glib::MainContext::default().block_on(super::super::local_file_identity(
        &gio::File::for_path(&source),
    ))?;
    fs::rename(&source, fixture.path().join("original"))?;
    fs::write(&source, b"replacement")?;
    let parent = std::sync::Arc::new(super::super::open_local_parent_directory(fixture.path())?);
    let result = glib::MainContext::default().block_on(copy_restore_entry(
        parent.clone(),
        "source".into(),
        parent,
        "destination".into(),
        identity,
        gio::Cancellable::new(),
        Rc::new(RefCell::new(Vec::new())),
    ));
    assert!(result.is_err());
    assert!(!fixture.path().join("destination").exists());
    assert_eq!(fs::read(source)?, b"replacement");
    assert_eq!(fs::read(fixture.path().join("original"))?, b"original");
    Ok(())
}

#[test]
fn restore_copy_rollback_preserves_foreign_and_replaced_entries() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let source = fixture.path().join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("contents"), b"original")?;
    let parent = std::sync::Arc::new(super::super::open_local_parent_directory(fixture.path())?);
    let journal = Rc::new(RefCell::new(Vec::new()));
    glib::MainContext::default().block_on(copy_restore_entry(
        parent.clone(),
        "source".into(),
        parent,
        "destination".into(),
        None,
        gio::Cancellable::new(),
        journal.clone(),
    ))?;
    let destination = fixture.path().join("destination");
    fs::write(destination.join("foreign"), b"unrelated")?;
    fs::rename(
        destination.join("contents"),
        fixture.path().join("old-copy"),
    )?;
    fs::write(destination.join("contents"), b"replacement")?;
    assert!(
        glib::MainContext::default()
            .block_on(discard_restore_copy(journal))
            .is_err()
    );
    assert_eq!(fs::read(destination.join("foreign"))?, b"unrelated");
    assert_eq!(fs::read(destination.join("contents"))?, b"replacement");
    assert_eq!(fs::read(source.join("contents"))?, b"original");
    Ok(())
}

#[test]
fn restore_directory_copy_preserves_permissions_times_and_xattrs() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let source = fixture.path().join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("contents"), b"original")?;
    fs::set_permissions(&source, fs::Permissions::from_mode(0o750))?;
    let source_file = fs::File::open(&source)?;
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    let accessed = SystemTime::UNIX_EPOCH + Duration::from_secs(1_500_000_000);
    source_file.set_times(
        fs::FileTimes::new()
            .set_accessed(accessed)
            .set_modified(modified),
    )?;
    rustix::fs::fsetxattr(
        &source_file,
        "user.strata-test",
        b"metadata",
        rustix::fs::XattrFlags::empty(),
    )?;
    let destination = fixture.path().join("destination");
    glib::MainContext::default().block_on(move_restore_path_with(
        source.clone(),
        destination.clone(),
        fixture.path().to_path_buf(),
        gio::Cancellable::new(),
        |_, _, _, _, _| Err(Errno::INVAL),
    ))?;
    assert!(!source.exists());
    assert_eq!(fs::metadata(&destination)?.mode() & 0o777, 0o750);
    assert_eq!(fs::metadata(&destination)?.modified()?, modified);
    assert_eq!(fs::metadata(&destination)?.accessed()?, accessed);
    let target = fs::File::open(destination)?;
    let mut value = [0u8; 32];
    let count = rustix::fs::fgetxattr(&target, "user.strata-test", &mut value[..])?;
    assert_eq!(&value[..count], b"metadata");
    Ok(())
}

#[test]
fn restore_refuses_a_destination_created_immediately_before_rename() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let source = fixture.path().join("source");
    let destination = fixture.path().join("destination");
    fs::write(&source, b"original")?;
    let raced = destination.clone();
    let result = glib::MainContext::default().block_on(move_restore_path_with(
        source.clone(),
        destination.clone(),
        fixture.path().to_path_buf(),
        gio::Cancellable::new(),
        move |from, name, to, target, flags| {
            fs::write(&raced, b"concurrent user data").expect("racing destination");
            rustix::fs::renameat_with(from, name, to, target, flags)
        },
    ));
    assert!(
        result
            .expect_err("collision")
            .message()
            .contains("already exists")
    );
    assert_eq!(fs::read(&source)?, b"original");
    assert_eq!(fs::read(&destination)?, b"concurrent user data");
    Ok(())
}

#[test]
fn restore_keeps_payload_and_metadata_when_destination_is_occupied() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    for directory in [false, true] {
        let fixture = tempfile::tempdir()?;
        let destination = fixture.path().join("report");
        if directory {
            fs::create_dir(&destination)?;
        } else {
            symlink(fixture.path().join("absent"), &destination)?;
        }
        let (source, entry) = volume_trash_entry(fixture.path(), "report", "report", b"original")?;
        let info = source
            .parent()
            .and_then(Path::parent)
            .expect("trash root")
            .join("info/report.trashinfo");
        let metadata = fs::read(&info)?;
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let _operation = LocalOperationProvider.restore(
            RestoreRequest {
                id: OperationRequestId(502),
                source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                    entry,
                    destination: destination.clone(),
                }]),
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );
        wait_for_restore(&events);
        assert!(
            matches!(events.borrow().last(), Some(OperationEvent::RestoreCompletedWithErrors { message, .. }) if message.contains("already exists")),
            "{:?}",
            events.borrow()
        );
        assert_eq!(fs::read(&source)?, b"original");
        assert_eq!(fs::read(&info)?, metadata);
        assert!(fs::symlink_metadata(&destination).is_ok());
    }
    Ok(())
}

#[test]
fn restore_execution_refuses_a_parent_symlink_escape() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let allowed = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let source = allowed.path().join("source");
    fs::write(&source, b"original")?;
    symlink(outside.path(), allowed.path().join("parent"))?;
    let result = glib::MainContext::default().block_on(move_restore_path(
        source.clone(),
        allowed.path().join("parent/report"),
        allowed.path().to_path_buf(),
        gio::Cancellable::new(),
    ));
    assert!(result.is_err());
    assert_eq!(fs::read(&source)?, b"original");
    assert!(!outside.path().join("report").exists());
    Ok(())
}

#[test]
fn restore_shared_trash_moves_the_item_and_removes_its_metadata() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let trash = fixture
        .path()
        .join(".Trash")
        .join(rustix::process::getuid().as_raw().to_string());
    fs::create_dir_all(trash.join("files"))?;
    fs::create_dir_all(trash.join("info"))?;
    fs::create_dir(fixture.path().join("Documents"))?;
    let source = trash.join("files/report");
    let info = trash.join("info/report.trashinfo");
    fs::write(&source, b"original")?;
    fs::write(&info, "[Trash Info]\nPath=Documents/report\n")?;
    let destination = fixture.path().join("Documents/report");
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(502),
            source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry: file_entry(&source),
                destination: destination.clone(),
            }]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_restore(&events);
    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::Restored { .. })
        ),
        "{:?}",
        events.borrow()
    );
    assert_eq!(fs::read(destination)?, b"original");
    assert!(!source.exists());
    assert!(!info.exists());
    assert!(!fixture.path().join(".Trash/Documents/report").exists());
    Ok(())
}

#[test]
fn home_trash_fallback_finds_broken_symlinks_the_virtual_backend_has_not_refreshed()
-> Result<(), Box<dyn Error>> {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let fixture = std::env::temp_dir().join(format!("strata-home-trash-fallback-{unique}"));
    let trash = fixture.join("Trash");
    let original = fixture.join("original report.txt");
    fs::create_dir_all(trash.join("files"))?;
    fs::create_dir_all(trash.join("info"))?;
    std::os::unix::fs::symlink("missing-target", trash.join("files/report.txt"))?;
    let encoded = original.display().to_string().replace(' ', "%20");
    fs::write(
        trash.join("info/report.txt.trashinfo"),
        format!("[Trash Info]\nPath={encoded}\nDeletionDate=2026-09-03T16:05:39\n"),
    )?;

    let entries = home_trash_entries_at(
        &trash,
        &HashSet::from([original.clone()]),
        &gio::Cancellable::new(),
        &HashMap::new(),
    );

    let entry = entries.get(&original).expect("fallback entry");
    assert_eq!(
        entry.source,
        Location::local(trash.join("files/report.txt"))
    );
    assert_eq!(entry.original_target, Some(Location::local(&original)));
    assert_eq!(
        entry.trash_info.as_deref(),
        Some(trash.join("info/report.txt.trashinfo").as_path())
    );
    let cancellable = gio::Cancellable::new();
    cancellable.cancel();
    assert!(
        home_trash_entries_at(
            &trash,
            &HashSet::from([original]),
            &cancellable,
            &HashMap::new()
        )
        .is_empty()
    );
    assert!(fs::symlink_metadata(trash.join("files/report.txt")).is_ok());
    assert!(trash.join("info/report.txt.trashinfo").exists());
    fs::remove_dir_all(fixture)?;
    Ok(())
}

#[test]
fn cancelling_restore_before_io_reports_every_item_as_unattempted() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let location = Location::local("/fixture/trashed.txt");
    for (source, expected) in [
        (
            RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry: file_entry(Path::new("/fixture/trashed.txt")),
                destination: PathBuf::from("/fixture/trashed.txt"),
            }]),
            vec![location.clone()],
        ),
        (
            RestoreSource::OriginalLocations(vec![location.clone()]),
            vec![location],
        ),
        (RestoreSource::OriginalLocations(Vec::new()), Vec::new()),
    ] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let operation = LocalOperationProvider.restore(
            RestoreRequest {
                id: OperationRequestId(9),
                source,
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );

        drop(operation);
        while events.borrow().is_empty() {
            glib::MainContext::default().iteration(true);
        }

        assert!(matches!(
            events.borrow().as_slice(),
            [OperationEvent::Cancelled { result, .. }]
                if result.completed.is_empty()
                    && result.failed.is_empty()
                    && result.not_attempted == expected
        ));
    }
    Ok(())
}

#[test]
fn restore_rejects_a_volume_orig_path_on_another_device() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let Some((home, stick)) = crate::test_support::distinct_device_dirs(
        "restore_rejects_a_volume_orig_path_on_another_device",
    ) else {
        return Ok(());
    };
    let dest = home.path().join(".config/autostart/payload.desktop");
    let (source, entry) = volume_trash_entry(
        stick.path(),
        "payload",
        &dest.to_string_lossy(),
        b"ssh-ed25519 AAAA attacker",
    )?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(478),
            source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry,
                destination: dest.clone(),
            }]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_restore(&events);

    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::RestoreCompletedWithErrors { message, .. })
                if message.contains("outside the trash volume")
        ),
        "{:?}",
        events.borrow()
    );
    assert!(source.exists());
    assert_eq!(fs::read(&source)?, b"ssh-ed25519 AAAA attacker");
    assert!(!dest.exists());
    Ok(())
}

#[test]
fn restore_returns_a_volume_item_to_a_path_on_the_same_volume() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let fixture = tempfile::tempdir()?;
    fs::create_dir_all(fixture.path().join("Documents"))?;
    let (source, entry) = volume_trash_entry(
        fixture.path(),
        "report.txt",
        "Documents/report.txt",
        b"notes",
    )?;
    let destination = fixture.path().canonicalize()?.join("Documents/report.txt");

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(479),
            source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry,
                destination: destination.clone(),
            }]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_restore(&events);

    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::Restored { .. })
        ),
        "{:?}",
        events.borrow()
    );
    assert_eq!(fs::read(&destination)?, b"notes");
    assert!(!source.exists());
    Ok(())
}

#[test]
fn restore_fails_when_the_confirmed_destination_no_longer_matches() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let fixture = tempfile::tempdir()?;
    fs::create_dir_all(fixture.path().join("Documents"))?;
    let (source, entry) = volume_trash_entry(
        fixture.path(),
        "report.txt",
        "Documents/report.txt",
        b"notes",
    )?;
    let confirmed = fixture.path().canonicalize()?.join("Documents/other.txt");

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(480),
            source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry,
                destination: confirmed.clone(),
            }]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_restore(&events);

    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::RestoreCompletedWithErrors { message, .. })
                if message.contains("no longer matches the confirmed destination")
        ),
        "{:?}",
        events.borrow()
    );
    assert!(source.exists());
    assert!(!confirmed.exists());
    assert!(!fixture.path().join("Documents/report.txt").exists());
    Ok(())
}

#[test]
fn restore_uses_the_trash_entry_target_path_as_the_physical_source() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let fixture = tempfile::tempdir()?;
    fs::create_dir_all(fixture.path().join("Documents"))?;
    let (source, mut entry) = volume_trash_entry(
        fixture.path(),
        "report.txt",
        "Documents/report.txt",
        b"notes",
    )?;
    entry.location = Location::uri("trash:///report.txt");
    entry.thumbnail_path = Some(source.clone());
    let destination = fixture.path().canonicalize()?.join("Documents/report.txt");

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(481),
            source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                entry,
                destination: destination.clone(),
            }]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_restore(&events);

    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::Restored { .. })
        ),
        "{:?}",
        events.borrow()
    );
    assert_eq!(fs::read(&destination)?, b"notes");
    assert!(!source.exists());
    Ok(())
}
