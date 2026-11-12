// SPDX-License-Identifier: MIT

//! Restores of Trash entries through the operation provider.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// A `report` folder holding `notes.txt` in a volume's Trash, deleted from `Documents`.
struct TrashedReport {
    item: PathBuf,
    info: PathBuf,
    destination: PathBuf,
}

impl TrashedReport {
    fn create(root: &Path) -> Result<Self, Box<dyn Error>> {
        let trash = VolumeTrash::create(root)?;
        let item = trash.file("report");
        fs::create_dir_all(&item)?;
        fs::create_dir(root.join("Documents"))?;
        fs::write(item.join("notes.txt"), b"notes")?;
        trash.add_dated_info("report", "Documents/report")?;
        Ok(Self {
            info: trash.info("report"),
            destination: root.canonicalize()?.join("Documents/report"),
            item,
        })
    }

    fn restore(&self, id: u64) -> Vec<OperationEvent> {
        restore_entries(id, vec![trashed_directory(&self.item, &self.destination)])
    }
}

/// Restores the staged `report` entry as a merge undo gives it back.
fn merge_undo_restores(report: &TrashedReport, emit: Rc<dyn Fn(OperationEvent)>) {
    let staged = RefCell::new(Some(RestoreEntry {
        source: Location::local(&report.item),
        display_name: "report".to_owned(),
        original_target: Some(Location::local(&report.destination)),
        trash_info: Some(report.info.clone()),
        confirmed_destination: None,
        physical_path: Some(report.item.clone()),
    }));
    let lookup: StagedOriginalLookup = Rc::new(move |_, _| {
        let staged = staged.borrow_mut().take();
        Box::pin(async move { staged.ok_or_else(|| io_error("staged once")) })
    });
    glib::MainContext::default().block_on(run_merge_undo(
        OperationRequestId(504),
        (
            Vec::new(),
            vec![Location::local(&report.destination)],
            HashMap::new(),
        ),
        emit,
        gio::Cancellable::new(),
        lookup,
    ));
}

#[test]
fn restore_leaves_in_the_trash_what_it_cannot_copy_or_finds_changed() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // Whether the Trash copy changes, and whether a merge undo restores it.
    for (changed, merge_undo) in [(false, false), (true, false), (false, true)] {
        let fixture = tempfile::tempdir()?;
        let report = TrashedReport::create(fixture.path())?;
        let (name, cause) = if changed {
            ("notes.txt", KeptCause::Changed)
        } else {
            std::os::unix::net::UnixListener::bind(report.item.join("socket"))?;
            ("socket", KeptCause::Skipped(SpecialKind::Socket))
        };
        // Rewritten in the Trash once it is copied, before the copy is flushed.
        let racer = writes(&report.item.join("notes.txt"), b"rewritten notes");
        let _scope = FsQuirks::install(
            fixture.path(),
            Quirks {
                mount_type: Some(("ext4", "rw")),
                before: if changed {
                    vec![(Op::SyncFs, "*", 0, racer)]
                } else {
                    Vec::new()
                },
                ..Quirks::without_placement()
            },
        );

        let events = if merge_undo {
            let (events, emit) = recording();
            merge_undo_restores(&report, emit);
            events.take()
        } else {
            report.restore(503)
        };

        assert_restored(&events, "");
        assert_eq!(
            kept_in_trash(&events),
            vec![KeptItem {
                location: Location::local(report.item.join(name)),
                cause,
            }]
        );
        expect_entries(&[&report.item.join(name)]);
        assert!(report.info.exists(), "the Trash entry stays restorable");
        expect_contents(&[(report.destination.join("notes.txt"), b"notes")]);
        expect_no_entry(&[&report.destination.join("socket")]);
        assert_eq!(report.item.join("notes.txt").exists(), changed);
    }
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

        let events = restore_entries(
            502,
            vec![RestoreTrashItem {
                entry,
                destination: destination.clone(),
            }],
        );

        assert_restore_failed(&events, "already exists");
        expect_contents(&[(source, b"original"), (info, &metadata)]);
        expect_entries(&[&destination]);
    }
    Ok(())
}

#[test]
fn restore_shared_trash_moves_the_item_and_removes_its_metadata() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let uid = rustix::process::getuid().as_raw();
    let trash = VolumeTrash::create_at(fixture.path().join(".Trash").join(uid.to_string()))?;
    fs::create_dir(fixture.path().join("Documents"))?;
    let source = trash.file("report");
    fs::write(&source, b"original")?;
    trash.add_info("report", "Documents/report")?;
    let destination = fixture.path().join("Documents/report");

    let events = restore_entries(502, vec![trashed_file(&source, &destination)]);

    assert_restored(&events, "");
    expect_contents(&[(destination, b"original")]);
    expect_gone(&[
        &source,
        &trash.info("report"),
        &fixture.path().join(".Trash/Documents/report"),
    ]);
    Ok(())
}

#[test]
fn home_trash_fallback_finds_broken_symlinks_the_virtual_backend_has_not_refreshed()
-> Result<(), Box<dyn Error>> {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let fixture = std::env::temp_dir().join(format!("strata-home-trash-fallback-{unique}"));
    let trash = VolumeTrash::create_at(fixture.join("Trash"))?;
    let original = fixture.join("original report.txt");
    symlink("missing-target", trash.file("report.txt"))?;
    let encoded = original.display().to_string().replace(' ', "%20");
    fs::write(
        trash.info("report.txt"),
        format!("[Trash Info]\nPath={encoded}\nDeletionDate=2026-09-03T16:05:39\n"),
    )?;
    let lookup = |cancellable: &gio::Cancellable| {
        trash_entries_at(
            std::slice::from_ref(&trash.trash),
            &HashSet::from([original.clone()]),
            cancellable,
            &HashMap::new(),
        )
    };

    let entries = lookup(&gio::Cancellable::new());

    let entry = entries
        .get(&original)
        .and_then(|found| found.as_ref().ok())
        .expect("fallback entry");
    assert_eq!(
        (
            entry.source.clone(),
            entry.original_target.clone(),
            entry.trash_info.clone()
        ),
        (
            Location::local(trash.file("report.txt")),
            Some(Location::local(&original)),
            Some(trash.info("report.txt"))
        )
    );
    let cancellable = gio::Cancellable::new();
    cancellable.cancel();
    assert!(lookup(&cancellable).is_empty());
    expect_entries(&[&trash.file("report.txt")]);
    assert!(trash.info("report.txt").exists());
    fs::remove_dir_all(fixture)?;
    Ok(())
}

#[test]
fn restore_rejects_a_volume_orig_path_on_another_device() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
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

    let events = restore_entries(
        478,
        vec![RestoreTrashItem {
            entry,
            destination: dest.clone(),
        }],
    );

    assert_restore_failed(&events, "outside the trash volume");
    assert!(source.exists());
    expect_contents(&[(source, b"ssh-ed25519 AAAA attacker")]);
    expect_gone(&[&dest]);
    Ok(())
}

/// A `report.txt` in the volume's Trash, deleted from `Documents`, with where it goes back.
fn trashed_volume_report(root: &Path) -> Result<(PathBuf, FileEntry, PathBuf), Box<dyn Error>> {
    fs::create_dir_all(root.join("Documents"))?;
    let (source, entry) = volume_trash_entry(root, "report.txt", "Documents/report.txt", b"notes")?;
    let destination = root.canonicalize()?.join("Documents/report.txt");
    Ok((source, entry, destination))
}

#[test]
fn restore_returns_a_volume_item_to_a_path_on_the_same_volume() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let fixture = tempfile::tempdir()?;
    let (source, entry, destination) = trashed_volume_report(fixture.path())?;

    let events = restore_entries(
        479,
        vec![RestoreTrashItem {
            entry,
            destination: destination.clone(),
        }],
    );

    assert_restored(&events, "");
    expect_contents(&[(destination, b"notes")]);
    expect_gone(&[&source]);
    Ok(())
}

#[test]
fn restore_fails_when_the_confirmed_destination_no_longer_matches() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let fixture = tempfile::tempdir()?;
    let (source, entry, destination) = trashed_volume_report(fixture.path())?;
    let confirmed = destination.with_file_name("other.txt");

    let events = restore_entries(
        480,
        vec![RestoreTrashItem {
            entry,
            destination: confirmed.clone(),
        }],
    );

    assert_restore_failed(&events, "no longer matches the confirmed destination");
    assert!(source.exists());
    expect_gone(&[&confirmed, &fixture.path().join("Documents/report.txt")]);
    Ok(())
}

#[test]
fn restore_uses_the_trash_entry_target_path_as_the_physical_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let fixture = tempfile::tempdir()?;
    let (source, mut entry, destination) = trashed_volume_report(fixture.path())?;
    entry.location = Location::uri("trash:///report.txt");
    entry.thumbnail_path = Some(source.clone());

    let events = restore_entries(
        481,
        vec![RestoreTrashItem {
            entry,
            destination: destination.clone(),
        }],
    );

    assert_restored(&events, "");
    expect_contents(&[(destination, b"notes")]);
    expect_gone(&[&source]);
    Ok(())
}

/// Counts each close of a `notes.txt` copy.
fn counting_closes(
    copied: &Arc<AtomicUsize>,
    count: usize,
) -> Vec<(Op, &'static str, usize, Racer)> {
    (0..count)
        .map(|_| {
            let copied = copied.clone();
            let racer: Racer = Box::new(move || {
                copied.fetch_add(1, Ordering::SeqCst);
            });
            (Op::Close, "notes.txt", 0, racer)
        })
        .collect()
}

/// Waits until both copies closed, or gives up after a while.
fn waits_for_both_copies(copied: &Arc<AtomicUsize>, gave_up: &Arc<AtomicBool>) -> Racer {
    let (both, giving_up) = (copied.clone(), gave_up.clone());
    Box::new(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        while both.load(Ordering::SeqCst) < 2 {
            if Instant::now() > deadline {
                giving_up.store(true, Ordering::SeqCst);
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
    })
}

#[test]
fn trash_items_whose_files_share_names_are_restored_whole() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let trash = VolumeTrash::create(fixture.path())?;
    let names = ["daily.0", "daily.1"];
    fs::create_dir(fixture.path().join("Backups"))?;
    for name in names {
        fs::create_dir_all(trash.file(name))?;
        trash.add_dated_info(name, &format!("Backups/{name}"))?;
    }
    let shared = trash.file("daily.0/notes.txt");
    fs::write(&shared, b"notes")?;
    fs::hard_link(&shared, trash.file("daily.1/notes.txt"))?;
    // Neither Trash copy is deleted before both items are copied.
    let copied = Arc::new(AtomicUsize::new(0));
    let gave_up = Arc::new(AtomicBool::new(false));
    let _scope = FsQuirks::install(
        fixture.path(),
        Quirks {
            mount_type: Some(("ext4", "rw")),
            before: vec![(Op::SyncFs, "*", 0, waits_for_both_copies(&copied, &gave_up))],
            after: counting_closes(&copied, names.len()),
            ..Quirks::without_placement()
        },
    );
    let destination = fixture.path().canonicalize()?.join("Backups");
    let items = names
        .iter()
        .map(|name| trashed_directory(&trash.file(name), &destination.join(name)))
        .collect();

    let events = restore_entries(507, items);

    assert!(
        !gave_up.load(Ordering::SeqCst),
        "a Trash copy was synced for deletion before both items were copied"
    );
    assert_restored(&events, "");
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, OperationEvent::ItemsReported { .. })),
        "{events:?}"
    );
    assert_eq!(
        (
            fs::read_dir(trash.files())?.count(),
            fs::read_dir(trash.trash.join("info"))?.count()
        ),
        (0, 0)
    );
    expect_contents(&names.map(|name| (destination.join(name).join("notes.txt"), &b"notes"[..])));
    Ok(())
}

#[test]
fn a_trash_entry_stays_restorable_while_its_item_is_in_the_trash() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // An entry added to the Trash item once the copy listed it, or an item
    // put at its name in the Trash once the restore removed it.
    for added in [true, false] {
        let fixture = tempfile::tempdir()?;
        let report = TrashedReport::create(fixture.path())?;
        let (before, after) = if added {
            let racer = writes(&report.item.join("added.txt"), b"added");
            (vec![(Op::SyncFs, "*", 0, racer)], Vec::new())
        } else {
            let racer = writes(&report.item, b"another");
            // The first removes the placeholder the placement ladder tried.
            (Vec::new(), vec![(Op::Unlink, "report", 1, racer)])
        };
        let _scope = FsQuirks::install(
            fixture.path(),
            Quirks {
                mount_type: Some(("ext4", "rw")),
                before,
                after,
                ..Quirks::without_placement()
            },
        );

        let events = report.restore(508);

        assert_restored(&events, &added.to_string());
        let expected = if added {
            vec![KeptItem {
                location: Location::local(&report.item),
                cause: KeptCause::Changed,
            }]
        } else {
            Vec::new()
        };
        assert_eq!(kept_in_trash(&events), expected, "{added}");
        expect_entries(&[&report.item]);
        assert!(
            report.info.exists(),
            "{added}: the Trash entry stays restorable"
        );
        expect_contents(&[(report.destination.join("notes.txt"), b"notes")]);
    }
    Ok(())
}
