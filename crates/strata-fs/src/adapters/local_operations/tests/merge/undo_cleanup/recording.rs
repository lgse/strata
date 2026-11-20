// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn what_an_operation_placed_is_read_back_only_until_its_tick_passes() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let epoch = SystemTime::UNIX_EPOCH;
    let seconds = SystemTime::now().duration_since(epoch)?.as_secs() / 2 * 2;
    for change in ["untouched", "rewritten where read"] {
        let _clock =
            ClockGuard::set(epoch + Duration::from_secs(seconds) + Duration::from_millis(500));
        let root = tempfile::tempdir()?;
        let archive = root.path().join("archive.zip");
        fs::write(&archive, b"0123456789abcdef")?;
        set_times(&archive, seconds, 0)?;
        let past_the_tick = epoch + Duration::from_secs(seconds + 3);
        let read_on = Arc::new(AtomicBool::new(false));
        let reading = read_on.clone();
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                ctime_is_mtime: true,
                short_io: Some(4),
                before: vec![
                    (
                        Op::Read,
                        "archive.zip",
                        1,
                        Box::new(move || {
                            super::super::super::super::copy_engine::set_clock_for_test(Some(
                                past_the_tick,
                            ));
                        }),
                    ),
                    (
                        Op::Read,
                        "archive.zip",
                        2,
                        Box::new(move || reading.store(true, Ordering::Release)),
                    ),
                ],
                ..Quirks::vfat()
            },
        );

        let placed = placed_now(&[&archive]);

        assert!(
            !read_on.load(Ordering::Acquire),
            "{change}: read past the tick"
        );
        if change == "rewritten where read" {
            fs::write(&archive, b"0123XXXX89abcdef")?;
            set_times(&archive, seconds, 0)?;
        }
        let original = staged_trash_fixture(root.path(), "archive.zip", &archive, b"original")?;
        let events = run_undo(
            (Vec::new(), vec![Location::local(&archive)], placed),
            staged_lookup(HashMap::from([(archive.clone(), original)])),
        );
        if change == "untouched" {
            assert_eq!(fs::read(&archive)?, b"original");
            assert!(kept_in_trash(&events).is_empty(), "{events:?}");
        } else {
            assert_eq!(fs::read(&archive)?, b"0123XXXX89abcdef");
            assert_eq!(kept_in_trash(&events), changed_after_the_merge(&archive));
        }
    }
    Ok(())
}

#[test]
fn an_undo_keeps_what_a_cancel_stopped_the_operation_from_recording() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let epoch = SystemTime::UNIX_EPOCH;
    let seconds = SystemTime::now().duration_since(epoch)?.as_secs() / 2 * 2;
    let _clock = ClockGuard::set(epoch + Duration::from_secs(seconds) + Duration::from_millis(500));
    for (folder, op, name) in [(false, Op::Read, "placed"), (true, Op::Stat, "sub")] {
        let root = tempfile::tempdir()?;
        let placed = root.path().join("placed");
        if folder {
            write_files(&placed, &[("a.txt", "a"), ("sub/b.txt", "b")])?;
        } else {
            fs::write(&placed, b"placed")?;
            set_times(&placed, seconds, 0)?;
        }
        let cancellable = gio::Cancellable::new();
        let cancel = cancellable.clone();
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                ctime_is_mtime: !folder,
                before: vec![(op, name, 0, Box::new(move || cancel.cancel()))],
                ..if folder {
                    Quirks::default()
                } else {
                    Quirks::vfat()
                }
            },
        );

        let recorded =
            glib::MainContext::default().block_on(super::super::super::super::copy_engine::placed(
                placed.clone(),
                super::super::super::super::copy_engine::placed_budget(),
                &cancellable,
            ));

        let location = Location::local(&placed);
        let original = staged_trash_fixture(root.path(), "placed", &placed, b"original")?;
        let events = run_undo(
            (
                Vec::new(),
                vec![location.clone()],
                recorded
                    .map(|recorded| HashMap::from([(location, recorded)]))
                    .unwrap_or_default(),
            ),
            staged_lookup(HashMap::from([(placed.clone(), original)])),
        );
        assert_eq!(kept_in_trash(&events), changed_after_the_merge(&placed));
        assert!(trash_in(root.path()).join("info/placed.trashinfo").exists());
        if folder {
            assert_eq!(names(&placed)?, ["a.txt", "sub"]);
        } else {
            assert_eq!(fs::read(&placed)?, b"placed");
        }
    }
    Ok(())
}

/// A replaced folder of four entries.
fn check_replaced_folder_too_big(root: &Path) -> Result<(), Box<dyn Error>> {
    let target = root.join("target");
    write_files(&target, &[("a.txt", "a"), ("sub/b.txt", "b")])?;
    let placed = placed_now(&[&target]);
    let original = staged_trash_folder(root, "target", &target, &[("old.txt", "old")])?;
    let events = run_undo(
        (Vec::new(), vec![Location::local(&target)], placed),
        staged_lookup(HashMap::from([(target.clone(), original)])),
    );
    assert_eq!(names(&target)?, ["a.txt", "sub"]);
    assert_kept_in_trash(&events, (&trash_in(root), "target"), &target, "");
    Ok(())
}

/// A merge's two overwritten files, which share the limit.
fn check_merged_files_too_big(root: &Path) -> Result<(), Box<dyn Error>> {
    let source = root.join("source");
    let target = root.join("target");
    write_files(&source, &[("a.txt", "incoming a"), ("b.txt", "incoming b")])?;
    write_files(&target, &[("a.txt", "old a"), ("b.txt", "old b")])?;
    let (stage, _) = rename_stage(root.join("staging"));
    let (plans, on_merged) = recorded_plans();
    merge_staged(&source, &target, stage, &on_merged)?;
    let [a, b] = ["a.txt", "b.txt"].map(|name| target.join(name));
    let staged = HashMap::from([
        (
            a.clone(),
            staged_trash_fixture(root, "a.txt", &a, b"old a")?,
        ),
        (
            b.clone(),
            staged_trash_fixture(root, "b.txt", &b, b"old b")?,
        ),
    ]);
    let events = run_undo(undo_parts(plans.take()), staged_lookup(staged));
    assert_files(&target, &[("a.txt", "old a"), ("b.txt", "incoming b")], "")?;
    assert_eq!(kept_in_trash(&events), changed_after_the_merge(&b));
    Ok(())
}

#[test]
fn an_undo_keeps_what_was_too_big_to_record() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (merged, limit) in [(false, 3), (true, 1)] {
        let _limit = PlacedEntriesGuard::set(limit);
        let root = tempfile::tempdir()?;
        if merged {
            check_merged_files_too_big(root.path())?;
        } else {
            check_replaced_folder_too_big(root.path())?;
        }
    }
    Ok(())
}

#[test]
fn a_created_tree_too_big_to_record_leaves_its_entries_to_the_rest() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let _limit = PlacedEntriesGuard::set(3);
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    write_files(
        &source,
        &[
            ("big/1.txt", "1"),
            ("big/2.txt", "2"),
            ("big/3.txt", "3"),
            ("z.txt", "incoming z"),
        ],
    )?;
    write_files(&target, &[("stays.txt", "keep me")])?;
    let _scope = FsQuirks::install(root.path(), without_trash(Quirks::default()));
    let (merged, plans) = merge_through_trash(&source, &target, false, real_copy());
    merged?;
    let [big, z] = ["big", "z.txt"].map(|name| target.join(name));
    let parts = undo_parts(plans);
    assert_eq!(placed_paths(&parts.2), std::slice::from_ref(&z));

    let events = run_undo(parts, staged_lookup(HashMap::new()));

    assert!(!z.exists(), "{events:?}");
    assert_eq!(names(&big)?, ["1.txt", "2.txt", "3.txt"]);
    assert_eq!(kept_items(&events), [Location::local(&big)]);
    Ok(())
}

#[test]
fn a_paste_records_no_more_than_its_limit_across_its_merges() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let _limit = PlacedEntriesGuard::set(1);
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    write_files(
        &sources,
        &[("a/first.txt", "first"), ("b/second.txt", "second")],
    )?;
    write_files(
        &destination,
        &[("a/stays.txt", "keep me"), ("b/stays.txt", "keep me")],
    )?;
    let _scope = FsQuirks::install(root.path(), without_trash(Quirks::default()));

    let events = run_paste(PasteRequest {
        id: OperationRequestId(1618),
        destination: Location::local(&destination),
        items: ["a", "b"]
            .map(|name| PasteItem {
                source: Location::local(sources.join(name)),
                conflict: TransferConflict::Merge,
                target_name: None,
            })
            .into(),
        move_sources: false,
    });

    let placed = events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::Merged { placed, .. } => Some(placed.clone()),
            _ => None,
        })
        .flatten()
        .collect::<HashMap<_, _>>();
    assert_eq!(placed_paths(&placed), [destination.join("a/first.txt")]);
    Ok(())
}

#[test]
fn a_merge_records_what_it_creates_only_where_undo_cannot_trash_it() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // A created folder of four entries sorts ahead of z.txt, with room to
    // record three.
    let _limit = PlacedEntriesGuard::set(3);
    for overwrites in [true, false] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        write_files(
            &source,
            &[
                ("big/1.txt", "1"),
                ("big/2.txt", "2"),
                ("big/3.txt", "3"),
                ("z.txt", "incoming z"),
            ],
        )?;
        write_files(&target, &[("stays.txt", "keep me")])?;
        let z = target.join("z.txt");
        if overwrites {
            fs::write(&z, b"old z")?;
        }
        // The Trash took the original, or is offered where there is none.
        let _scope = (!overwrites).then(|| {
            FsQuirks::install(
                root.path(),
                Quirks {
                    trash: Trash::Refused,
                    ..Quirks::default()
                },
            )
        });
        let (stage, _) = rename_stage(root.path().join("staging"));
        let (plans, on_merged) = recorded_plans();

        merge_staged(&source, &target, stage, &on_merged)?;

        let parts = undo_parts(plans.take());
        if !overwrites {
            assert!(parts.2.is_empty(), "{:?}", placed_paths(&parts.2));
            continue;
        }
        assert_eq!(placed_paths(&parts.2), std::slice::from_ref(&z));
        let staged = HashMap::from([(
            z.clone(),
            staged_trash_fixture(root.path(), "z.txt", &z, b"old z")?,
        )]);
        let events = run_undo((Vec::new(), parts.1, parts.2), staged_lookup(staged));
        assert_eq!(fs::read(&z)?, b"old z", "{events:?}");
    }
    Ok(())
}
