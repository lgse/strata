// SPDX-License-Identifier: MIT

use super::*;

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
    let copying = super::super::super::Copying {
        progress: Some(progress),
        names: super::super::super::NamePolicy::default(),
        ..Default::default()
    };

    let moved =
        glib::MainContext::default().block_on(super::super::super::move_local_with_progress(
            gio::File::for_path(&folder),
            gio::File::for_path(destination.join("folder")),
            Taken::Fail,
            cancellable,
            copying.clone(),
            super::super::super::attempt_move(None),
            super::super::super::remote_copy(copying),
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
    let remote_copy: super::super::super::RemoteCopy =
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

    let moved =
        glib::MainContext::default().block_on(super::super::super::move_local_with_progress(
            gio::File::for_path(&folder),
            target.clone(),
            Taken::Fail,
            gio::Cancellable::new(),
            super::super::super::Copying {
                progress: Some(progress),
                names: super::super::super::NamePolicy::default(),
                ..Default::default()
            },
            super::super::super::attempt_move(None),
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
    let remote_copy: super::super::super::RemoteCopy =
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

    glib::MainContext::default().block_on(super::super::super::move_local_with_progress(
        gio::File::for_path(&folder),
        target,
        Taken::Fail,
        gio::Cancellable::new(),
        super::super::super::Copying {
            progress: Some(progress),
            ..Default::default()
        },
        super::super::super::attempt_move(None),
        remote_copy,
    ))?;

    assert_eq!(fs::read(stand_in.join("a.txt"))?, b"a");
    assert_eq!(reported(&events).kept, []);
    assert!(names(&share)?.is_empty());
    Ok(())
}
