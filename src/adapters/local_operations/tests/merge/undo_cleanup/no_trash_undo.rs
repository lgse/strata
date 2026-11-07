// SPDX-License-Identifier: MIT

use super::*;

/// What happened to the created `new.txt` and `sub` after the merge,
/// whether undo has Trash, and whether the mount places folders or copies
/// them in place.
#[derive(Clone, Copy)]
struct Created {
    change: &'static str,
    trash: Trash,
    places_folders: bool,
}

impl Created {
    fn row(self) -> String {
        let Self {
            change,
            trash,
            places_folders,
        } = self;
        format!("{change}, {trash:?}, places folders: {places_folders}")
    }

    fn install(self, root: &Path) -> FsQuirks {
        let placement = if self.places_folders {
            Quirks::default()
        } else {
            Quirks {
                mount_type: Some(("ext4", "rw")),
                ..Quirks::without_folder_placement()
            }
        };
        let scope = FsQuirks::install(
            root,
            Quirks {
                trash: self.trash,
                ..placement
            },
        );
        if !self.places_folders {
            learn_mount_capability(scope.key(), |capabilities| {
                capabilities.no_replace = Some(false);
                capabilities.hard_links = Some(true);
                capabilities.rename_over_empty_directory = Some(false);
            });
        }
        scope
    }

    fn change_after_the_merge(
        self,
        file: &Path,
        folder: &Path,
        parts: &mut UndoParts,
    ) -> io::Result<()> {
        match self.change {
            "edited" => {
                fs::write(file, b"edited after the merge")?;
                fs::write(folder.join("a.txt"), b"edited after the merge")?;
            }
            "added to" => fs::write(folder.join("c.txt"), b"added after the merge")?,
            "unrecorded" => parts.2.clear(),
            _ => {}
        }
        Ok(())
    }

    /// What undo keeps of the created `file` and `folder`.
    fn kept<'a>(self, file: &'a Path, folder: &'a Path) -> Vec<&'a Path> {
        match (self.change, self.trash) {
            (_, Trash::Supported) | ("untouched", _) => Vec::new(),
            ("added to", _) => vec![folder],
            _ => vec![file, folder],
        }
    }
}

fn trashed_locations(events: &[OperationEvent]) -> Vec<Location> {
    let mut trashed = trashed_items(events)
        .into_iter()
        .map(|(location, _)| location)
        .collect::<Vec<_>>();
    trashed.sort_by_key(Location::display_name);
    trashed
}

/// Undo kept what `kept` names, reporting each, and deleted the rest of
/// the created `items`.
fn assert_kept_reported(events: &[OperationEvent], items: [&Path; 2], kept: &[&Path], row: &str) {
    let Some(OperationEvent::CompletedWithErrors {
        deleted_locations,
        message,
        ..
    }) = events.last()
    else {
        panic!("{row}: {events:?}");
    };
    let deleted: Vec<_> = items
        .into_iter()
        .filter(|item| !kept.contains(item))
        .map(Location::local)
        .collect();
    assert_eq!(*deleted_locations, deleted, "{row}");
    let reason = crate::i18n::tr("It was kept, as it may have changed since it was copied");
    for item in kept {
        let line =
            super::super::super::super::item_error(&Location::local(item).display_name(), &reason);
        assert!(message.contains(&line), "{row}: {message}");
    }
}

fn check_created_undo(case: Created) -> Result<(), Box<dyn Error>> {
    let row = case.row();
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    write_files(
        &source,
        &[
            ("docs/new.txt", "incoming"),
            ("docs/sub/a.txt", "incoming a"),
            ("docs/sub/b.txt", "incoming b"),
        ],
    )?;
    write_files(&target, &[("docs/stays.txt", "keep me")])?;
    let _scope = case.install(root.path());
    let (merged, plans) = merge_through_trash(&source, &target, false, real_copy());
    merged?;
    let [file, folder] = ["docs/new.txt", "docs/sub"].map(|path| target.join(path));
    let mut parts = undo_parts(plans);
    case.change_after_the_merge(&file, &folder, &mut parts)?;

    let events = run_undo(parts, staged_lookup(HashMap::new()));

    assert_eq!(
        fs::read(target.join("docs/stays.txt"))?,
        b"keep me",
        "{row}"
    );
    let trashed = trashed_locations(&events);
    if matches!(case.trash, Trash::Supported) {
        assert_eq!(
            trashed,
            [Location::local(&file), Location::local(&folder)],
            "{row}: Trash takes them whatever they hold"
        );
    } else {
        assert!(trashed.is_empty(), "{row}");
    }
    let kept = case.kept(&file, &folder);
    for item in [&file, &folder] {
        assert_eq!(
            item.exists(),
            kept.contains(&item.as_path()),
            "{row}: {item:?}"
        );
    }
    let mut kept_items = kept_items(&events);
    kept_items.sort_by_key(Location::display_name);
    assert_eq!(
        kept_items,
        kept.iter().map(Location::local).collect::<Vec<_>>(),
        "{row}: nothing is left to undo of what was kept"
    );
    if kept.contains(&folder.as_path()) {
        assert_kept_whole(&folder, case.change, &row)?;
    }
    if kept.is_empty() {
        assert!(
            matches!(events.last(), Some(OperationEvent::Restored { .. })),
            "{row}: {events:?}"
        );
    } else {
        assert_kept_reported(&events, [&file, &folder], &kept, &row);
    }
    Ok(())
}

fn assert_kept_whole(folder: &Path, change: &str, row: &str) -> io::Result<()> {
    let mut expected = vec!["a.txt", "b.txt"];
    if change == "added to" {
        expected.push("c.txt");
    }
    assert_eq!(names(folder)?, expected, "{row}: kept whole");
    assert_eq!(fs::read(folder.join("b.txt"))?, b"incoming b", "{row}");
    Ok(())
}

#[test]
fn a_merge_undo_without_trash_deletes_only_what_the_merge_created_unchanged()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let created = |change, trash, places_folders| Created {
        change,
        trash,
        places_folders,
    };
    let rows = [
        created("untouched", Trash::Unsupported, true),
        created("untouched", Trash::Unsupported, false),
        created("edited", Trash::Unsupported, true),
        created("added to", Trash::Unsupported, true),
        created("unrecorded", Trash::Unsupported, true),
        created("edited", Trash::Supported, true),
    ];
    for case in rows {
        check_created_undo(case)?;
    }
    Ok(())
}

/// Racers that replace, or edit in place, the copied `notes.txt` and
/// `plain/a.txt` as soon as they are published.
fn arrivals_before_recording(
    root: &Path,
    destination: &Path,
    published: Op,
    in_place: bool,
) -> io::Result<Vec<(Op, &'static str, usize, Racer)>> {
    let mut after = Vec::new();
    for (op, name, relative) in [
        (published, "notes.txt", "notes.txt"),
        (Op::Rename, "plain", "plain/a.txt"),
    ] {
        let saved = root.join(format!("saved-{name}"));
        fs::write(&saved, b"concurrent arrival")?;
        let over = destination.join(relative);
        let racer: Racer = Box::new(move || {
            if in_place {
                fs::write(&over, b"concurrent arrival").expect("edit copied entry");
            } else {
                fs::rename(&saved, &over).expect("replace copied entry");
            }
        });
        after.push((op, name, 0, racer));
    }
    Ok(after)
}

/// As the app gathers the undo of a paste with a merge.
fn paste_undo_parts(events: Vec<OperationEvent>) -> UndoParts {
    let mut parts = UndoParts::default();
    for event in events {
        match event {
            OperationEvent::TransferProgress {
                created_location, ..
            } => parts.0.extend(created_location),
            OperationEvent::Merged {
                created, placed, ..
            } => {
                parts.0.extend(created);
                parts.2.extend(placed);
            }
            _ => {}
        }
    }
    parts
}

fn paste_item(sources: &Path, name: &str, conflict: TransferConflict) -> PasteItem {
    PasteItem {
        source: Location::local(sources.join(name)),
        conflict,
        target_name: None,
    }
}

fn pasted_copies_fixture(root: &Path) -> io::Result<(PathBuf, PathBuf)> {
    let sources = root.join("sources");
    let destination = root.join("destination");
    write_files(
        &sources,
        &[
            ("notes.txt", "notes"),
            ("docs/new.txt", "incoming"),
            ("keep.txt", "incoming keep"),
            ("plain/a.txt", "a"),
        ],
    )?;
    write_files(
        &destination,
        &[("docs/stays.txt", "keep me"), ("keep.txt", "original keep")],
    )?;
    Ok((sources, destination))
}

fn check_pasted_copies_undo(
    (stage, published, tmpfile): (&str, Op, Support),
    change: &str,
) -> Result<(), Box<dyn Error>> {
    let context = format!("{change}, {stage}");
    let edited = change != "unchanged";
    let root = tempfile::tempdir()?;
    let (sources, destination) = pasted_copies_fixture(root.path())?;
    let racing = change.ends_with("before recording");
    let after = if racing {
        let in_place = change == "edited before recording";
        arrivals_before_recording(root.path(), &destination, published, in_place)?
    } else {
        Vec::new()
    };
    let _scope = FsQuirks::install(
        root.path(),
        without_trash(Quirks {
            after,
            tmpfile,
            ..Quirks::default()
        }),
    );

    let events = run_paste(PasteRequest {
        id: OperationRequestId(1619),
        destination: Location::local(&destination),
        items: vec![
            paste_item(&sources, "notes.txt", TransferConflict::FailIfExists),
            paste_item(&sources, "docs", TransferConflict::Merge),
            paste_item(&sources, "keep.txt", TransferConflict::KeepBoth),
            paste_item(&sources, "plain", TransferConflict::FailIfExists),
        ],
        move_sources: false,
    });
    let parts = paste_undo_parts(events);
    let [notes, plain] = ["notes.txt", "plain"].map(|name| destination.join(name));
    if change == "edited" {
        fs::write(&notes, b"edited after the paste")?;
        fs::write(plain.join("a.txt"), b"edited after the paste")?;
    }

    let undone = run_undo(parts, staged_lookup(HashMap::new()));

    let mut left = vec!["docs", "keep.txt"];
    let mut kept = Vec::new();
    if edited {
        left.extend(["notes.txt", "plain"]);
        kept.extend([Location::local(&notes), Location::local(&plain)]);
    }
    assert_eq!(names(&destination)?, left, "{context}: {undone:?}");
    assert_eq!(names(&destination.join("docs"))?, ["stays.txt"]);
    assert_eq!(fs::read(destination.join("keep.txt"))?, b"original keep");
    let mut kept_items = kept_items(&undone);
    kept_items.sort_by_key(Location::display_name);
    assert_eq!(kept_items, kept, "{context}");
    if racing {
        let arrivals = [
            ("notes.txt", "concurrent arrival"),
            ("plain/a.txt", "concurrent arrival"),
        ];
        assert_files(&destination, &arrivals, &context)?;
    }
    Ok(())
}

#[test]
fn a_merge_undo_without_trash_deletes_what_its_paste_copied_only_if_unchanged()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // A file is published by linking its unnamed copy, or by renaming its named stage.
    let stages = [
        ("unnamed file", Op::LinkFd, Support::Supported),
        (
            "named stage",
            Op::Rename,
            Support::Rejected(Errno::OPNOTSUPP),
        ),
    ];
    let changes = [
        "unchanged",
        "edited",
        "replaced before recording",
        "edited before recording",
    ];
    for (stage, change) in stages
        .into_iter()
        .flat_map(|stage| changes.map(|change| (stage, change)))
    {
        if stage.1 == Op::LinkFd && !base_offers(&UNNAMED_FILES)? {
            continue;
        }
        check_pasted_copies_undo(stage, change)?;
    }
    Ok(())
}

#[test]
fn a_created_folder_changed_while_undo_deletes_it_fails_as_not_empty() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("sub");
    write_files(&folder, &[("a.txt", "a"), ("b.txt", "b")])?;
    let placed = placed_now(&[&folder]);
    let added = folder.join("c.txt");
    let racer_added = added.clone();
    let racer: Racer = Box::new(move || {
        fs::write(&racer_added, b"added during the undo").expect("added entry");
    });
    let _scope = FsQuirks::install(
        root.path(),
        without_trash(Quirks {
            after: vec![(Op::Unlink, "a.txt", 0, racer)],
            ..Quirks::default()
        }),
    );

    let events = run_undo(
        (vec![Location::local(&folder)], Vec::new(), placed),
        staged_lookup(HashMap::new()),
    );

    assert_eq!(names(&folder)?, ["c.txt"]);
    assert!(
        kept_items(&events).is_empty(),
        "what is left is still to undo"
    );
    let Some(OperationEvent::CompletedWithErrors {
        deleted_locations,
        message,
        ..
    }) = events.last()
    else {
        panic!("{events:?}");
    };
    assert!(deleted_locations.is_empty());
    let not_empty = crate::services::io_error_detail(&io::Error::from_raw_os_error(
        Errno::NOTEMPTY.raw_os_error(),
    ));
    assert!(
        message.contains(&super::super::super::super::item_error("sub", &not_empty)),
        "{message}"
    );
    Ok(())
}
