// SPDX-License-Identifier: MIT

//! Undoing moves: conflicts, flushes, cancels and group folder cleanup.

use super::*;

/// A root with an `origin` folder and a `second` folder items were moved to.
fn origin_and(second: &str) -> io::Result<(tempfile::TempDir, PathBuf, PathBuf)> {
    let root = tempfile::tempdir()?;
    let origin = root.path().join("origin");
    let other = root.path().join(second);
    fs::create_dir_all(&origin)?;
    fs::create_dir_all(&other)?;
    Ok((root, origin, other))
}

#[test]
fn undoing_a_move_returns_each_item_to_its_original_directory() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for quirks in [Quirks::default, Quirks::sshfs, Quirks::vboxsf] {
        let (root, origin, archive) = origin_and("archive")?;
        let _scope = FsQuirks::install(root.path(), quirks());
        let moved = archive.join("report.txt");
        fs::write(&moved, b"contents")?;
        let original = inode(&moved);
        let item = fail_if_exists(record(&origin.join("report.txt"), &moved));

        let events = undo_move(undo_request(40, vec![item]));

        assert_pasted(&events, "");
        expect_names(&[(&archive, &[]), (&origin, &["report.txt"])])?;
        assert_eq!(inode(&origin.join("report.txt")), original);
        expect_contents(&[(origin.join("report.txt"), b"contents")]);
    }
    Ok(())
}

#[test]
fn undoing_a_move_stops_at_an_unconfirmed_conflict() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let (_root, origin, archive) = origin_and("archive")?;
    let blocked = origin.join("report.txt");
    fs::write(&blocked, b"newer")?;
    let first = archive.join("notes.txt");
    let second = archive.join("report.txt");
    fs::write(&first, b"first")?;
    fs::write(&second, b"second")?;
    let items = vec![
        fail_if_exists(record(&origin.join("notes.txt"), &first)),
        fail_if_exists(record(&blocked, &second)),
    ];

    let events = undo_move(undo_request(41, items));

    assert_eq!(
        completed_before_failing(&events),
        vec![Location::local(&first)]
    );
    assert_eq!(
        item_moves(&events),
        [(
            Location::local(&first),
            Location::local(origin.join("notes.txt"))
        )]
    );
    expect_contents(&[
        (blocked, b"newer"),
        (second, b"second"),
        (origin.join("notes.txt"), b"first"),
    ]);
    Ok(())
}

#[test]
fn a_confirmed_undo_conflict_replaces_the_newer_item() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let (_root, origin, archive) = origin_and("archive")?;
    let original = origin.join("report.txt");
    let moved = archive.join("report.txt");
    fs::write(&original, b"newer")?;
    fs::write(&moved, b"moved")?;
    let item = UndoMoveItem {
        record: record(&original, &moved),
        conflict: TransferConflict::ReplaceExisting,
    };

    let events = undo_move(undo_request(42, vec![item]));

    assert_pasted(&events, "");
    expect_gone(&[&moved]);
    expect_contents(&[(original, b"moved")]);
    Ok(())
}

/// Records whether every watched path still exists each time it runs.
fn records_all_present(during: &Arc<std::sync::Mutex<Vec<bool>>>, watched: &[PathBuf]) -> Racer {
    let (record, watched) = (during.clone(), watched.to_vec());
    Box::new(move || {
        record
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(watched.iter().all(|current| current.exists()));
    })
}

fn undo_items(id: u64, moved: &[(PathBuf, PathBuf)]) -> UndoMoveRequest {
    let items = moved
        .iter()
        .map(|(original, current)| fail_if_exists(record(original, current)))
        .collect();
    undo_request(id, items)
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
        let moved: Vec<(PathBuf, PathBuf)> = names
            .iter()
            .map(|name| (origin.join(name), archive.join(name)))
            .collect();
        for (_, current) in &moved {
            fs::write(current, b"contents")?;
        }
        // The items share one barrier, before which every moved item stays.
        let during = Arc::new(std::sync::Mutex::new(Vec::new()));
        let currents: Vec<PathBuf> = moved.iter().map(|(_, current)| current.clone()).collect();
        let _archive = FsQuirks::install(&archive, Quirks::default());
        let _origin = FsQuirks::install(
            &origin,
            Quirks {
                before: vec![(Op::SyncFs, "*", 0, records_all_present(&during, &currents))],
                fail: if flushed {
                    Vec::new()
                } else {
                    vec![(Op::SyncFs, "*", 0, Errno::IO)]
                },
                ..Quirks::ext4()
            },
        );

        let (_operation, events) = start_undo_move(undo_items(id, &moved));
        pump_until_transfer(&events);

        assert_eq!(
            *during.lock().unwrap_or_else(|error| error.into_inner()),
            [true],
            "flushed: {flushed}"
        );
        for (original, _) in &moved {
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

/// Files, a folder with a file, and a symlink and named pipe in each folder.
fn seed_origin(origin: &Path) -> io::Result<()> {
    fs::create_dir_all(origin.join("project/sub"))?;
    fs::write(origin.join("notes.txt"), b"notes")?;
    fs::write(origin.join("project/sub/b.txt"), b"b")?;
    for folder in [origin.to_path_buf(), origin.join("project")] {
        std::os::unix::fs::symlink("elsewhere", folder.join("link"))?;
        rustix::fs::mknodat(
            rustix::fs::CWD,
            folder.join("pipe"),
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::from_raw_mode(0o600),
            0,
        )?;
    }
    Ok(())
}

fn paste_items(
    id: u64,
    items: &[&str],
    from: &Path,
    to: &Path,
    move_sources: bool,
) -> PasteRequest {
    PasteRequest {
        id: OperationRequestId(id),
        destination: Location::local(to),
        items: items
            .iter()
            .map(|item| PasteItem {
                source: Location::local(from.join(item)),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            })
            .collect(),
        move_sources,
    }
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
    seed_origin(&origin)?;
    fs::create_dir(&share)?;
    fs::create_dir(&viewer)?;
    let scope = FsQuirks::install(&share, preset());
    run_paste(paste_items(1592, items, &origin, &share, true));
    if let Some(read_after) = read_after {
        super::super::super::copy_engine::set_clock_for_test(Some(started + read_after));
        run_paste(paste_items(1593, items, &share, &viewer, false));
    }
    land_deferred_closes(&share)?;
    super::super::super::copy_engine::set_clock_for_test(Some(started + undone_after));
    scope.clear_calls();
    let moved: Vec<(PathBuf, PathBuf)> = items
        .iter()
        .map(|item| (origin.join(item), share.join(item)))
        .collect();

    let (_operation, events) = start_undo_move(undo_items(1594, &moved));
    pump_until_transfer(&events);

    let events = events.take();
    assert_pasted(&events, &row);
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

/// Pumps the main context until the move back ends, serving the cancel from the main
/// thread, and returns every event.
fn drive_until_cancelled(
    events: Rc<RefCell<Vec<OperationEvent>>>,
    cancel: &CancelFromMain,
    mut operation: Option<LoadHandle>,
) -> Vec<OperationEvent> {
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
    events.take()
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
    let items = names
        .map(|name| fail_if_exists(record(&originals.join(name), &moved.join(name))))
        .into();
    let (operation, events) = start_undo_move(undo_request(1615, items));

    let events = drive_until_cancelled(events, &cancel, Some(operation));

    let result = cancelled_result(&events, "cut off move back");
    assert_eq!(
        outcome(result),
        (
            Vec::new(),
            Vec::new(),
            names.map(|name| Location::local(moved.join(name))).to_vec()
        )
    );
    assert_eq!(fs::read_dir(&originals)?.count(), 0);
    for name in names {
        assert_eq!(fs::read(moved.join(name))?.len(), 4 * 1024 * 1024);
    }
    Ok(())
}

#[test]
fn undoing_a_grouping_reports_the_trashed_group_folder() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::undo::moves::undoing_a_grouping_reports_the_trashed_group_folder",
        || {
            let root = tempfile::tempdir().expect("root");
            let group = root.path().join("group");
            fs::create_dir(&group).expect("group folder");
            fs::write(group.join("report.txt"), b"contents").expect("grouped item");
            let identity = numbers_at(&group);
            let item = fail_if_exists(record(
                &root.path().join("report.txt"),
                &group.join("report.txt"),
            ));

            let events = undo_move(UndoMoveRequest {
                cleanup_locations: vec![Location::local(&group)],
                ..undo_request(42, vec![item])
            });

            assert_pasted(&events, "");
            expect_gone(&[&group]);
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
        let item = fail_if_exists(record(
            &root.path().join("report.txt"),
            &group.join("report.txt"),
        ));

        let events = undo_move(UndoMoveRequest {
            cleanup_locations: vec![Location::local(&group)],
            ..undo_request(43, vec![item])
        });

        assert_pasted(&events, "");
        assert!(trashed_items(&events).is_empty());
        expect_contents(&[(root.path().join("report.txt"), b"contents")]);
        assert_eq!(group.exists(), arrived_meanwhile);
    }
    Ok(())
}
