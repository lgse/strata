// SPDX-License-Identifier: MIT

use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

use super::fs_quirks::{Op, Racer, Silly, Trash};
use super::*;
use crate::services::file_activity;

const LONG: Duration = Duration::from_secs(10);

struct Preview {
    stopped: Arc<AtomicUsize>,
    intact_when_stopped: Arc<Mutex<Option<bool>>>,
    close: mpsc::Sender<()>,
    reader: Option<thread::JoinHandle<()>>,
}

impl Preview {
    fn of(path: &Path) -> Self {
        use std::os::unix::fs::MetadataExt;
        let file = fs::File::open(path).expect("open the previewed file");
        let inode = file.metadata().expect("previewed file").ino();
        let (close, closed) = mpsc::channel();
        let stopped = Arc::new(AtomicUsize::new(0));
        let intact_when_stopped = Arc::new(Mutex::new(None));
        let (counted, stop) = (stopped.clone(), close.clone());
        let hold = file_activity::hold([path], move || {
            counted.fetch_add(1, Ordering::SeqCst);
            let _ = stop.send(());
        })
        .expect("a preview outside any operation");
        let (seen, intact, path) = (
            stopped.clone(),
            intact_when_stopped.clone(),
            path.to_path_buf(),
        );
        let reader = thread::spawn(move || {
            let _ = closed.recv_timeout(LONG);
            if seen.load(Ordering::SeqCst) > 0 {
                let same = fs::symlink_metadata(&path).is_ok_and(|now| now.ino() == inode);
                *intact.lock().expect("preview state") = Some(same);
            }
            drop(file);
            drop(hold);
        });
        Self {
            stopped,
            intact_when_stopped,
            close,
            reader: Some(reader),
        }
    }

    fn was_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst) > 0
    }

    fn stopped_before_the_change(mut self) -> bool {
        let _ = self.close.send(());
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        *self.intact_when_stopped.lock().expect("preview state") == Some(true)
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        let _ = self.close.send(());
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn settled(event: &OperationEvent) -> bool {
    matches!(
        event,
        OperationEvent::Renamed { .. }
            | OperationEvent::Pasted { .. }
            | OperationEvent::TransferFailed { .. }
            | OperationEvent::Deleted { .. }
            | OperationEvent::CompletedWithErrors { .. }
            | OperationEvent::Restored { .. }
            | OperationEvent::RestoreCompletedWithErrors { .. }
            | OperationEvent::Compressed { .. }
            | OperationEvent::Cancelled { .. }
            | OperationEvent::Failed { .. }
    )
}

fn delete(folder: &Path) -> Vec<OperationEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(1729),
            entries: vec![directory_entry(folder)],
            permanent: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_operation(&events, |event| {
        matches!(
            event,
            OperationEvent::Deleted { .. }
                | OperationEvent::CompletedWithErrors { .. }
                | OperationEvent::Cancelled { .. }
        )
    });
    events.take()
}

#[test]
fn an_operation_releases_a_preview_of_a_file_in_the_folder_it_changes() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let nfs = || Quirks {
        silly_rename: Some(Silly::nfs()),
        ..Quirks::default()
    };
    for row in ["delete", "move to another disk"] {
        let root = tempfile::tempdir()?;
        let share = root.path().join("share");
        let folder = share.join("folder");
        fs::create_dir_all(&folder)?;
        fs::write(folder.join("song.ogg"), b"held")?;
        let destination = root.path().join("disk");
        fs::create_dir(&destination)?;
        let _share = FsQuirks::install(&share, nfs());
        let _disk = FsQuirks::install(&destination, Quirks::default());
        let preview = Preview::of(&folder.join("song.ogg"));

        let events = match row {
            "delete" => delete(&folder),
            _ => run_paste(PasteRequest {
                id: OperationRequestId(1729),
                destination: Location::local(&destination),
                items: vec![PasteItem {
                    source: Location::local(&folder),
                    conflict: TransferConflict::FailIfExists,
                    target_name: None,
                }],
                move_sources: true,
            }),
        };

        assert!(preview.stopped_before_the_change(), "{row}: {events:?}");
        assert!(
            !folder.exists(),
            "{row}: {:?}, left {:?}",
            events.last(),
            names(&folder)
        );
        assert!(names(&share)?.is_empty(), "{row}");
        match row {
            "delete" => assert!(
                matches!(events.last(), Some(OperationEvent::Deleted { .. })),
                "{row}: {events:?}"
            ),
            _ => assert_eq!(fs::read(destination.join("folder/song.ogg"))?, b"held"),
        }
    }
    Ok(())
}

#[test]
fn a_release_on_a_mount_that_does_not_answer_fences_lexically() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    fs::create_dir(&folder)?;
    fs::write(folder.join("a.txt"), b"a")?;
    let (unpark, parked) = mpsc::channel::<()>();
    let parked = Mutex::new(parked);
    let racer: Racer = Box::new(move || {
        let _ = parked
            .lock()
            .expect("parked server")
            .recv_timeout(Duration::from_secs(30));
    });
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(Op::OpenPath, "*", 0, racer)],
            ..Quirks::default()
        },
    );
    let preview = Preview::of(&folder.join("a.txt"));
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let started = Instant::now();
    let _operation = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(1729),
            entries: vec![directory_entry(&folder)],
            permanent: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    let context = glib::MainContext::default();
    while !preview.was_stopped() {
        assert!(started.elapsed() < LONG, "the preview was never stopped");
        context.iteration(false);
        thread::sleep(Duration::from_millis(1));
    }
    let refused = file_activity::hold([folder.join("b.txt")], || {}).is_err();

    wait_for_operation(&events, |event| {
        matches!(
            event,
            OperationEvent::Deleted { .. } | OperationEvent::CompletedWithErrors { .. }
        )
    });
    let took = started.elapsed();
    let _ = unpark.send(());

    assert!(refused, "a new read under the folder waits for the delete");
    assert!(
        matches!(events.borrow().last(), Some(OperationEvent::Deleted { .. })),
        "{:?}",
        events.borrow()
    );
    assert!(!folder.exists());
    assert!(
        took < super::super::release::RELEASE_BUDGET + Duration::from_secs(3),
        "the delete waited {took:?} on a parked lookup"
    );
    Ok(())
}

#[test]
fn a_merge_fences_an_original_only_when_it_is_set_aside() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for named_by_uri in [false, true] {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        for (folder, files) in [
            (
                &sources,
                &[("run/a.txt", "new a"), ("run/sub/new.txt", "new")][..],
            ),
            (
                &destination,
                &[
                    ("run/a.txt", "old a"),
                    ("run/b.txt", "old b"),
                    ("run/sub/old.txt", "old"),
                ][..],
            ),
        ] {
            for (path, contents) in files {
                let path = folder.join(path);
                fs::create_dir_all(path.parent().expect("parent"))?;
                fs::write(path, contents)?;
            }
        }
        let pasted_into = if named_by_uri {
            Location::uri(gio::File::for_path(&destination).uri())
        } else {
            Location::local(&destination)
        };
        let original = Preview::of(&destination.join("run/a.txt"));
        let sibling = Preview::of(&destination.join("run/b.txt"));
        let stopped_before_aside = Arc::new(Mutex::new(None));
        let target_active = Arc::new(Mutex::new(None));
        let nested_active = Arc::new(Mutex::new(None));
        let (seen, seen_active) = (stopped_before_aside.clone(), target_active.clone());
        let stopped = original.stopped.clone();
        let target = destination.join("run");
        let racer: Racer = Box::new(move || {
            *seen.lock().expect("set-aside check") = Some(stopped.load(Ordering::SeqCst) > 0);
            *seen_active.lock().expect("set-aside check") =
                Some(file_activity::is_active_in(&target));
        });
        let (seen_nested, nested, folder) = (
            nested_active.clone(),
            destination.join("run/sub"),
            pasted_into.clone(),
        );
        let nested_racer: Racer = Box::new(move || {
            *seen_nested.lock().expect("nested check") = Some((
                file_activity::is_active_in(&nested),
                file_activity::is_active_at(&folder),
            ));
        });
        let _sources = FsQuirks::install(&sources, Quirks::default());
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                trash: Trash::Unsupported,
                before: vec![
                    (Op::Rename, "a.txt", 0, racer),
                    (Op::Rename, "new.txt", 0, nested_racer),
                ],
                ..Quirks::default()
            },
        );

        run_paste(PasteRequest {
            id: OperationRequestId(1729),
            destination: pasted_into,
            items: vec![PasteItem {
                source: Location::local(sources.join("run")),
                conflict: TransferConflict::Merge,
                target_name: None,
            }],
            move_sources: false,
        });

        assert_eq!(
            *stopped_before_aside.lock().expect("set-aside check"),
            Some(true),
            "the original's preview stops before it is set aside"
        );
        assert_eq!(
            *target_active.lock().expect("set-aside check"),
            Some(true),
            "the merge target is active while the merge writes into it"
        );
        assert_eq!(
            *nested_active.lock().expect("nested check"),
            Some((true, true)),
            "a folder the merge descends into, and the destination named by URI ({named_by_uri}), \
             are active while the merge writes there"
        );
        assert!(
            !sibling.was_stopped(),
            "the rest of the target stays previewable"
        );
        assert_eq!(fs::read(destination.join("run/a.txt"))?, b"new a");
        assert_eq!(fs::read(destination.join("run/b.txt"))?, b"old b");
        assert_eq!(fs::read(destination.join("run/sub/new.txt"))?, b"new");
    }
    Ok(())
}

#[test]
fn every_operation_releases_a_preview_of_what_it_changes_before_changing_it()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let rows = [
        "rename",
        "rename through a linked folder",
        "undo a rename",
        "undo a move",
        "undo a move over a file with Replace",
        "undo a move out of a folder it removes",
        "copy over a file with Replace",
        "undo a copy",
        "undo a merge",
        "restore from Trash",
        "compress over an archive",
        "restore what an interrupted operation set aside",
    ];
    for row in rows {
        let root = tempfile::tempdir()?;
        let root = root.path();
        let _scope = FsQuirks::install(
            root,
            Quirks {
                trash: Trash::Unsupported,
                ..Quirks::default()
            },
        );
        let fixture = match row {
            "undo a move over a file with Replace" | "undo a move out of a folder it removes" => {
                Some(root.join("moved/a.txt"))
            }
            "copy over a file with Replace" => Some(root.join("copied/a.txt")),
            _ => None,
        };
        if let Some(fixture) = &fixture {
            fs::create_dir_all(fixture.parent().expect("parent"))?;
            fs::write(fixture, b"new")?;
        }
        let item = match row {
            "undo a move" => root.join("moved/a.txt"),
            "undo a move over a file with Replace" => root.join("c.txt"),
            "undo a move out of a folder it removes" => root.join("moved/other.txt"),
            "restore from Trash" => {
                let original = root.join("c.txt");
                volume_trash_entry(root, "a.txt", &original.to_string_lossy(), b"a")?.0
            }
            "compress over an archive" => root.join("archive.zip"),
            "restore what an interrupted operation set aside" => {
                root.join(HiddenName::Originals.fresh()).join("a.txt")
            }
            _ => root.join("a.txt"),
        };
        if !item.exists() {
            fs::create_dir_all(item.parent().expect("parent"))?;
            fs::write(&item, b"a")?;
        }
        fs::write(root.join("b.txt"), b"b")?;
        let preview = if row == "rename through a linked folder" {
            std::os::unix::fs::symlink(root, root.join("link"))?;
            Preview::of(&item.canonicalize()?)
        } else {
            Preview::of(&item)
        };
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let emit: Rc<dyn Fn(OperationEvent)> =
            Rc::new(move |event| emitted.borrow_mut().push(event));
        let id = OperationRequestId(1729);
        let elsewhere = Location::local(root.join("c.txt"));
        let _operation = match row {
            "rename" | "rename through a linked folder" => LocalOperationProvider.rename(
                crate::services::RenameRequest {
                    id,
                    entry: file_entry(&if row == "rename" {
                        item.clone()
                    } else {
                        root.join("link/a.txt")
                    }),
                    new_name: "c.txt".to_owned(),
                },
                emit,
            ),
            "undo a rename" => LocalOperationProvider.undo_rename(
                UndoRenameRequest {
                    id,
                    current: Location::local(&item),
                    original: elsewhere,
                },
                emit,
            ),
            "undo a move"
            | "undo a move over a file with Replace"
            | "undo a move out of a folder it removes" => LocalOperationProvider.undo_move(
                UndoMoveRequest {
                    id,
                    items: vec![UndoMoveItem {
                        record: MoveRecord {
                            original: elsewhere,
                            current: Location::local(fixture.as_ref().unwrap_or(&item)),
                            renamed: Vec::new(),
                        },
                        conflict: if row == "undo a move over a file with Replace" {
                            TransferConflict::ReplaceExisting
                        } else {
                            TransferConflict::FailIfExists
                        },
                    }],
                    cleanup_locations: if row == "undo a move out of a folder it removes" {
                        vec![Location::local(root.join("moved"))]
                    } else {
                        Vec::new()
                    },
                    names: NameMode::Keep,
                },
                emit,
            ),
            "copy over a file with Replace" => LocalOperationProvider.paste(
                PasteRequest {
                    id,
                    destination: Location::local(root),
                    items: vec![PasteItem {
                        source: Location::local(root.join("copied/a.txt")),
                        conflict: TransferConflict::ReplaceExisting,
                        target_name: None,
                    }],
                    move_sources: false,
                },
                emit,
            ),
            "undo a copy" => LocalOperationProvider.undo_copy(
                crate::services::UndoCopyRequest {
                    id,
                    locations: vec![Location::local(&item)],
                },
                emit,
            ),
            "undo a merge" => LocalOperationProvider.undo_merge(
                crate::services::UndoMergeRequest {
                    id,
                    created: vec![Location::local(&item)],
                    overwritten: Vec::new(),
                    originals: HashMap::new(),
                    placed: placed_now(&[&item]),
                },
                emit,
            ),
            "restore from Trash" => LocalOperationProvider.restore(
                RestoreRequest {
                    id,
                    source: RestoreSource::TrashEntries(vec![RestoreTrashItem {
                        entry: file_entry(&item),
                        destination: root.join("c.txt"),
                    }]),
                },
                emit,
            ),
            "restore what an interrupted operation set aside" => LocalOperationProvider.recover(
                crate::services::RecoverRequest {
                    id,
                    container: Location::local(item.parent().expect("its folder")),
                },
                emit,
            ),
            _ => LocalOperationProvider.compress(
                crate::services::CompressRequest {
                    id,
                    entries: vec![file_entry(&root.join("b.txt"))],
                    destination: Location::local(root),
                    archive_name: "archive".to_owned(),
                    conflict: TransferConflict::ReplaceExisting,
                    format: crate::services::ArchiveFormat::Zip,
                    password: None,
                },
                emit,
            ),
        };
        wait_for_operation(&events, settled);

        assert!(
            preview.stopped_before_the_change(),
            "{row}: {:?}",
            events.borrow()
        );
    }
    Ok(())
}

#[test]
fn restoring_an_item_gvfs_lists_in_a_volume_trash_releases_it() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let root = root.path();
    let original = root.join("music/song.ogg");
    let (trashed, _) = volume_trash_entry(root, "song.ogg", &original.to_string_lossy(), b"held")?;
    let info = gio::FileInfo::new();
    info.set_display_name("song.ogg");
    info.set_attribute_string(
        gio::FILE_ATTRIBUTE_STANDARD_TARGET_URI,
        &gio::File::for_path(&trashed).uri(),
    );
    let listed = Location::uri(format!(
        "trash:///{}",
        trashed.to_string_lossy().replace('/', "%5C")
    ));
    let entry = super::super::restore::gvfs_trash_entry(listed, &info, &original);
    let preview = Preview::of(&trashed);

    let operation = glib::MainContext::default().block_on(super::super::restore::release_trashed(
        &[entry],
        &gio::Cancellable::new(),
    ));

    assert!(preview.was_stopped(), "the trashed item's preview stops");
    assert!(
        file_activity::is_active_in(&root.join("music")),
        "the folder it goes back to is active"
    );
    drop(operation);
    Ok(())
}
