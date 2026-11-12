// SPDX-License-Identifier: MIT

use std::io::ErrorKind;

use super::*;

#[test]
fn a_refused_admin_prompt_is_not_authorized_while_other_denials_stay_errors() {
    let denied = || glib::Error::new(gio::IOErrorEnum::PermissionDenied, "Permission denied");

    assert_eq!(
        uri_validation_result(&Location::uri("admin:///root"), Err(denied())),
        Err(LocationValidationError::NotAuthorized)
    );
    assert!(matches!(
        uri_validation_result(&Location::uri("sftp://host/root"), Err(denied())),
        Err(LocationValidationError::Unavailable(_))
    ));
}

#[test]
fn validation_accepts_readable_directories_and_rejects_files_and_missing_paths() {
    let directory = FixtureDir::new("location");
    let file = directory.write("file.txt", b"fixture");

    for (location, expected) in [
        (directory.location(), Ok(())),
        (
            Location::local(&file),
            Err(LocationValidationError::NotDirectory),
        ),
        (
            Location::local(file.join("child")),
            Err(LocationValidationError::NotDirectory),
        ),
        (
            Location::local(directory.path().join("missing")),
            Err(LocationValidationError::Missing),
        ),
    ] {
        assert_eq!(
            LocalFileSource.validate_location(&location),
            expected,
            "{location:?}"
        );
    }

    directory.remove();
}

#[test]
fn permission_errors_are_reported_as_inaccessible() {
    let error = std::io::Error::from(ErrorKind::PermissionDenied);
    assert_eq!(
        LocationValidationError::from(error),
        LocationValidationError::Inaccessible
    );
}

#[test]
fn a_local_folder_is_validated_on_a_worker_while_the_window_keeps_running()
-> Result<(), Box<dyn Error>> {
    use crate::adapters::{FsOp, FsQuirks, Quirks};

    let _serial = serial();
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    fs::create_dir(&folder)?;
    let gate = Gate::default();
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(FsOp::FolderStamp, "folder", 0, gate.racer())],
            ..Quirks::default()
        },
    );
    let answer = Rc::new(RefCell::new(None));
    let slot = answer.clone();
    let ticked = Rc::new(Cell::new(false));
    let tick = ticked.clone();
    glib::timeout_add_local_once(Duration::from_millis(10), move || tick.set(true));

    let _validation = LocalFileSource.validate_location_async(
        Location::local(&folder),
        Rc::new(move |result| {
            slot.replace(Some(result));
        }),
    );
    drain_until(|| gate.reached() && ticked.get());
    assert!(answer.borrow().is_none());
    gate.release();
    drain_until(|| answer.borrow().is_some());

    assert_eq!(answer.take(), Some(Ok(())));
    Ok(())
}
