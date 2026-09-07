// SPDX-License-Identifier: GPL-3.0-or-later

use super::super::{move_restore_path, move_restore_path_with};
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
                message.contains("does not support atomic no-replace"),
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
