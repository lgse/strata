// SPDX-License-Identifier: MIT

use super::*;

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
    let rows: [(&str, super::super::fs_quirks::Preset, SourceMeanwhile); 9] = [
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
        let racer: Option<super::super::fs_quirks::Racer> = match meanwhile {
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
            super::super::super::fs_ops::fs().unlink(
                super::super::super::fs_ops::Entry::new(&dir, OsStr::new("open.txt")),
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
        let racer: super::super::fs_quirks::Racer = Box::new(move || {
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
            trash: super::super::fs_quirks::Trash::Unsupported,
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
