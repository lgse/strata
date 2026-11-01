// SPDX-License-Identifier: MIT

use super::*;

/// The item a move with `conflict` selects, and the file it brings.
fn incoming_for(conflict: TransferConflict) -> (&'static Path, &'static Path) {
    if conflict == TransferConflict::Merge {
        (Path::new("folder"), Path::new("folder/incoming.txt"))
    } else {
        (Path::new("notes.txt"), Path::new("notes.txt"))
    }
}

/// Puts at `destination` what the item meets with `conflict`.
fn meeting(conflict: TransferConflict, destination: &Path, item: &Path) -> io::Result<()> {
    match conflict {
        TransferConflict::ReplaceExisting => fs::write(destination.join(item), b"old")?,
        TransferConflict::Merge => {
            fs::create_dir(destination.join(item))?;
            fs::write(destination.join("folder/stays.txt"), b"keep")?;
        }
        _ => {}
    }
    Ok(())
}

/// Records whether `watched` still exists each time the destination is
/// flushed.
fn exists_at_flush(watched: PathBuf) -> (Arc<std::sync::Mutex<Vec<bool>>>, Racer) {
    let during = Arc::new(std::sync::Mutex::new(Vec::new()));
    let record = during.clone();
    let racer: Racer = Box::new(move || {
        record
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(watched.exists());
    });
    (during, racer)
}

fn check_flushed_move(
    id: u64,
    conflict: TransferConflict,
    flushed: bool,
) -> Result<(), Box<dyn Error>> {
    let row = format!("{conflict:?}, flushed: {flushed}");
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let destination = root.path().join("usb");
    let (item, incoming) = incoming_for(conflict);
    fs::create_dir_all(source_dir.join(incoming).parent().ok_or("a parent")?)?;
    fs::create_dir_all(&destination)?;
    fs::write(source_dir.join(incoming), b"incoming")?;
    meeting(conflict, &destination, item)?;
    let source = source_dir.join(item);
    let (during, racer) = exists_at_flush(source.clone());
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
        assert_pasted(&events, &row);
        assert!(!source.exists(), "{row}");
        assert!(moved, "{row}");
        return Ok(());
    }
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
    Ok(())
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
            check_flushed_move(id, conflict, flushed)?;
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
            trash: super::super::fs_quirks::Trash::Unsupported,
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
        let racer = |taken: PathBuf| -> Racer {
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
