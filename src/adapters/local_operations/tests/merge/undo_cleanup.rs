// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_merge_undo_deletes_only_what_the_merge_left_unchanged() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for change in [
        "untouched",
        "on a share",
        "linked",
        "edited",
        "touched",
        "replaced",
    ] {
        let root = tempfile::tempdir()?;
        let _share =
            (change == "on a share").then(|| FsQuirks::install(root.path(), Quirks::cifs_posix()));
        let source = root.path().join("source");
        let target = root.path().join("target");
        write_files(
            &source,
            &[("docs/a.txt", "incoming a"), ("docs/b.txt", "incoming b")],
        )?;
        if change == "linked" {
            fs::remove_file(source.join("docs/b.txt"))?;
            fs::hard_link(source.join("docs/a.txt"), source.join("docs/b.txt"))?;
        }
        write_files(
            &target,
            &[
                ("docs/a.txt", "old a"),
                ("docs/b.txt", "old b"),
                ("docs/stays.txt", "keep me"),
            ],
        )?;
        let (stage, _) = rename_stage(root.path().join("staging"));
        let (plans, on_merged) = recorded_plans();
        glib::MainContext::default().block_on(merge_local_with(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            false,
            gio::Cancellable::new(),
            None,
            MergeHooks {
                names: NamePolicy::default(),
                copy_into_target: real_copy(),
                stage_overwrite: stage,
                on_merged: &on_merged,
                progress: None,
            },
        ))?;
        let [a, b] = ["docs/a.txt", "docs/b.txt"].map(|path| target.join(path));
        match change {
            "linked" => assert_eq!(fs::metadata(&a)?.nlink(), 2, "the merge keeps the link"),
            "edited" => fs::write(&a, b"edited after the merge")?,
            "touched" => set_times(&a, 1_000_000_000, 0)?,
            "replaced" => {
                let merged = fs::metadata(&a)?;
                let other = a.with_file_name("a.new");
                fs::write(&other, fs::read(&a)?)?;
                set_times(
                    &other,
                    u64::try_from(merged.mtime())?,
                    u32::try_from(merged.mtime_nsec())?,
                )?;
                fs::rename(&other, &a)?;
            }
            _ => {}
        }
        let staged = HashMap::from([
            (
                a.clone(),
                staged_trash_fixture(root.path(), "a.txt", &a, b"old a")?,
            ),
            (
                b.clone(),
                staged_trash_fixture(root.path(), "b.txt", &b, b"old b")?,
            ),
        ]);
        let trash = trash_in(root.path());

        let events = run_undo(undo_parts(plans.take()), staged_lookup(staged));

        assert_eq!(fs::read(&b)?, b"old b", "{change}");
        assert_eq!(fs::read(target.join("docs/stays.txt"))?, b"keep me");
        if matches!(change, "untouched" | "on a share" | "linked") {
            assert_eq!(fs::read(&a)?, b"old a", "{change}");
            let mut undone = restored(&events).ok_or("not restored")?;
            undone.sort_by_key(Location::display_name);
            assert_eq!(undone, [Location::local(&a), Location::local(&b)]);
            assert!(kept_in_trash(&events).is_empty());
            continue;
        }
        let expected: &[u8] = match change {
            "edited" => b"edited after the merge",
            _ => b"incoming a",
        };
        assert_eq!(fs::read(&a)?, expected, "{change}");
        assert_eq!(
            restored(&events),
            Some(vec![Location::local(&b)]),
            "{change}"
        );
        assert_eq!(
            kept_in_trash(&events),
            changed_after_the_merge(&a),
            "{change}"
        );
        assert_eq!(fs::read(trash.join("files/a.txt"))?, b"old a", "{change}");
        assert!(trash.join("info/a.txt.trashinfo").exists(), "{change}");
    }
    Ok(())
}

#[test]
fn undoing_a_replace_deletes_what_it_placed_only_if_all_of_it_is_unchanged()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let rows = [
        (false, "untouched"),
        (false, "edited"),
        (false, "untouched on a share"),
        (false, "untouched under cached change times"),
        (true, "untouched"),
        (true, "edited"),
        (true, "added to"),
        (true, "edited during the undo"),
        (true, "undeletable"),
        (true, "still open in another program"),
    ];
    for (directory, change) in rows {
        let row = format!("{change} {}", if directory { "folder" } else { "file" });
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        if directory {
            write_files(&source, &[("a.txt", "new a"), ("sub/b.txt", "new b")])?;
            write_files(&target, &[("old.txt", "old")])?;
        } else {
            fs::write(&source, b"new")?;
            fs::write(&target, b"old")?;
        }
        // Placed items are recorded with a forced stat on any mount, not only
        // on a share this process just wrote to.
        let _cached = match change {
            "untouched on a share" => Some(Quirks::cifs_posix()),
            "untouched under cached change times" => Some(Quirks {
                cached_change_times: Some(Duration::from_secs(1)),
                ..Quirks::default()
            }),
            _ => None,
        }
        .map(|quirks| FsQuirks::install(root.path(), quirks));
        let replaced = RefCell::new(None);
        glib::MainContext::default().block_on(replace_local_with(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            false,
            gio::Cancellable::new(),
            None,
            super::super::super::Copying::default(),
            &|original, placed| *replaced.borrow_mut() = Some((original, placed)),
        ))?;
        let Some((ReplacedOriginal::Trashed(_), Some(placed))) = replaced.take() else {
            return Err(format!("{row}: the original was not replaced through Trash").into());
        };
        match (directory, change) {
            (false, "edited") => fs::write(&target, b"edited")?,
            (true, "edited") => fs::write(target.join("sub/b.txt"), b"edited b")?,
            (true, "added to") => fs::write(target.join("sub/c.txt"), b"added")?,
            _ => {}
        }
        let original = if directory {
            staged_trash_folder(root.path(), "target", &target, &[("old.txt", "old")])?
        } else {
            staged_trash_fixture(root.path(), "target", &target, b"old")?
        };
        let trash = trash_in(root.path());
        let location = Location::local(&target);
        let edited = target.join("sub/b.txt");
        let _scope = match change {
            "undeletable" => Some(Quirks {
                fail: vec![(Op::Unlink, "a.txt", 0, Errno::ACCESS)],
                ..Quirks::default()
            }),
            "still open in another program" => Some(Quirks {
                mount_type: Some(("nfs4", "rw,vers=4.2")),
                silly_rename: Some(super::super::fs_quirks::Silly::nfs()),
                holder: Some(crate::services::Holder::Program("Text Editor".to_owned())),
                ..Quirks::default()
            }),
            // Once the comparison found it unchanged, before its deletion.
            "edited during the undo" => Some(Quirks {
                before: vec![(
                    Op::Stat,
                    "b.txt",
                    1,
                    Box::new(move || drop(fs::write(&edited, b"edited b"))),
                )],
                ..Quirks::default()
            }),
            _ => None,
        }
        .map(|quirks| FsQuirks::install(root.path(), quirks));
        let _clock = (change == "still open in another program")
            .then(|| ClockGuard::set(std::time::SystemTime::now()));
        let _held =
            (change == "still open in another program").then(|| hold(&target.join("sub/b.txt")));

        let undo = || {
            run_undo(
                (
                    Vec::new(),
                    vec![location.clone()],
                    HashMap::from([(location.clone(), placed.clone())]),
                ),
                staged_lookup(HashMap::from([(target.clone(), original.clone())])),
            )
        };

        let events = undo();

        if change == "still open in another program" {
            let Some(OperationEvent::CompletedWithErrors { message, .. }) = events.last() else {
                panic!("{row}: {events:?}");
            };
            let error = rust_i18n::t!(
                "Could not delete “%{name}”: %{error}",
                name = "sub",
                error = rust_i18n::t!(
                    "A file in it is still open in %{program}. Close it there and try again.",
                    program = "Text Editor"
                )
            );
            assert!(message.contains(&*error), "{message}");
            assert!(kept_in_trash(&events).is_empty());
            assert!(trash.join("info/target.trashinfo").exists());
            continue;
        }
        if change == "undeletable" {
            let Some(OperationEvent::CompletedWithErrors { message, .. }) = events.last() else {
                panic!("{row}: {events:?}");
            };
            let denied = crate::services::io_error_detail(&io::Error::from_raw_os_error(
                Errno::ACCESS.raw_os_error(),
            ));
            let error = rust_i18n::t!(
                "Could not delete “%{name}”: %{error}",
                name = "a.txt",
                error = denied
            );
            assert!(message.contains(&*error), "{message}");
            assert_eq!(names(&target)?, ["a.txt"]);
            assert!(kept_in_trash(&events).is_empty());
            assert!(trash.join("info/target.trashinfo").exists());
            assert_eq!(restored(&undo()), Some(vec![location]));
            assert_eq!(names(&target)?, ["old.txt"]);
            continue;
        }
        if change.starts_with("untouched") {
            assert_eq!(restored(&events), Some(vec![location]), "{row}");
            let restored = if directory {
                assert_eq!(names(&target)?, ["old.txt"], "{row}");
                fs::read(target.join("old.txt"))?
            } else {
                fs::read(&target)?
            };
            assert_eq!(restored, b"old", "{row}");
            assert!(!trash.join("info/target.trashinfo").exists(), "{row}");
            continue;
        }
        assert_eq!(restored(&events), Some(Vec::new()), "{row}");
        assert_eq!(
            kept_in_trash(&events),
            changed_after_the_merge(&target),
            "{row}"
        );
        assert!(trash.join("info/target.trashinfo").exists(), "{row}");
        match (directory, change) {
            (false, _) => assert_eq!(fs::read(&target)?, b"edited"),
            (true, "edited during the undo") => {
                assert_eq!(fs::read(target.join("sub/b.txt"))?, b"edited b");
            }
            (true, _) => {
                assert_eq!(names(&target)?, ["a.txt", "sub"], "{row}");
                assert_eq!(fs::read(target.join("a.txt"))?, b"new a", "{row}");
                let sub = target.join("sub");
                if change == "edited" {
                    assert_eq!(names(&sub)?, ["b.txt"]);
                    assert_eq!(fs::read(sub.join("b.txt"))?, b"edited b");
                } else {
                    assert_eq!(names(&sub)?, ["b.txt", "c.txt"]);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn a_merge_undo_compares_what_it_placed_as_a_move_compares_its_sources()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let epoch = SystemTime::UNIX_EPOCH;
    let seconds = SystemTime::now().duration_since(epoch)?.as_secs() / 2 * 2;
    let cifs = |options| {
        move || Quirks {
            mount_type: Some(("cifs", options)),
            ..Quirks::default()
        }
    };
    let coarse = || Quirks {
        ctime_is_mtime: true,
        ..Quirks::vfat()
    };
    let unreadable = move || Quirks {
        fail: vec![(Op::OpenSource, "merged.txt", 0, Errno::ACCESS)],
        ..coarse()
    };
    // Inode numbers that change once the kernel evicts an entry, and a
    // rewrite within one tick of coarse times, which leaves the size and
    // times as they were.
    type MakeQuirks = Box<dyn Fn() -> Quirks>;
    let rows: [(&str, MakeQuirks, &str, bool); 6] = [
        (
            "cifs noserverino",
            Box::new(cifs("rw,vers=3.1.1")),
            "evicted",
            true,
        ),
        (
            "cifs",
            Box::new(cifs("rw,vers=3.1.1,serverino")),
            "evicted",
            false,
        ),
        ("vfat", Box::new(coarse), "untouched", true),
        ("vfat", Box::new(coarse), "rewritten", false),
        ("vfat", Box::new(coarse), "rewritten in a folder", false),
        ("vfat", Box::new(unreadable), "unread", false),
    ];
    for (mount, quirks, change, undone) in rows {
        let row = format!("{mount}, {change}");
        // Waiting for a tick to pass moves it on.
        let _clock =
            ClockGuard::set(epoch + Duration::from_secs(seconds) + Duration::from_millis(500));
        let root = tempfile::tempdir()?;
        let name = if change == "rewritten in a folder" {
            "folder"
        } else {
            "merged.txt"
        };
        let placed_at = root.path().join(name);
        let merged = if name == "folder" {
            fs::create_dir(&placed_at)?;
            placed_at.join("merged.txt")
        } else {
            placed_at.clone()
        };
        fs::write(&merged, b"merged")?;
        set_times(&merged, seconds, 0)?;
        let _scope = FsQuirks::install(root.path(), quirks());
        let placed = placed_now(&[&placed_at]);
        match change {
            "evicted" => super::super::fs_quirks::renumbered(&merged),
            "rewritten" | "rewritten in a folder" => {
                fs::write(&merged, b"MERGED")?;
                set_times(&merged, seconds, 0)?;
            }
            _ => {}
        }
        let original = staged_trash_fixture(root.path(), name, &placed_at, b"original")?;

        let events = run_undo(
            (Vec::new(), vec![Location::local(&placed_at)], placed),
            staged_lookup(HashMap::from([(placed_at.clone(), original)])),
        );

        assert!(restored(&events).is_some(), "{row}: {events:?}");
        if undone {
            assert_eq!(fs::read(&merged)?, b"original", "{row}");
            assert!(kept_in_trash(&events).is_empty(), "{row}");
        } else {
            assert_ne!(fs::read(&merged)?, b"original", "{row}");
            assert_eq!(
                kept_in_trash(&events),
                changed_after_the_merge(&placed_at),
                "{row}"
            );
            assert!(
                trash_in(root.path())
                    .join(format!("info/{name}.trashinfo"))
                    .exists(),
                "{row}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_merge_undo_cancelled_while_it_compares_an_item_leaves_it_whole() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let epoch = SystemTime::UNIX_EPOCH;
    let seconds = SystemTime::now().duration_since(epoch)?.as_secs() / 2 * 2;
    for after_a_kept_item in [false, true] {
        let _clock =
            ClockGuard::set(epoch + Duration::from_secs(seconds) + Duration::from_millis(500));
        let root = tempfile::tempdir()?;
        let merged = root.path().join("merged.txt");
        let edited = root.path().join("edited.txt");
        for path in [&merged, &edited] {
            fs::write(path, b"merged")?;
            set_times(path, seconds, 0)?;
        }
        let cancellable = gio::Cancellable::new();
        let cancel = cancellable.clone();
        let cancel: Racer = Box::new(move || cancel.cancel());
        // The first open is the record's own, the second the comparison's.
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                ctime_is_mtime: true,
                before: vec![(Op::OpenSource, "merged.txt", 1, cancel)],
                ..Quirks::vfat()
            },
        );
        let placed = placed_now(&[&merged, &edited]);
        fs::write(&edited, b"edited after the merge")?;
        let mut overwritten = vec![Location::local(&merged)];
        let mut staged = HashMap::from([(
            merged.clone(),
            staged_trash_fixture(root.path(), "merged.txt", &merged, b"original")?,
        )]);
        if after_a_kept_item {
            overwritten.insert(0, Location::local(&edited));
            staged.insert(
                edited.clone(),
                staged_trash_fixture(root.path(), "edited.txt", &edited, b"original")?,
            );
        }
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();

        glib::MainContext::default().block_on(run_merge_undo(
            OperationRequestId(1616),
            (Vec::new(), overwritten, placed),
            Rc::new(move |event| emitted.borrow_mut().push(event)),
            cancellable,
            staged_lookup(staged),
        ));

        let events = events.borrow();
        let Some(OperationEvent::Cancelled { result, .. }) = events.last() else {
            panic!("expected a cancel, got {events:?}");
        };
        let kept: &[Location] = if after_a_kept_item {
            &[Location::local(&edited)]
        } else {
            &[]
        };
        assert_eq!(result.completed, kept);
        assert!(result.failed.is_empty());
        assert_eq!(result.not_attempted, [Location::local(&merged)]);
        assert_eq!(fs::read(&merged)?, b"merged");
        assert_eq!(
            fs::read(trash_in(root.path()).join("files/merged.txt"))?,
            b"original"
        );
    }
    Ok(())
}

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
                            super::super::super::copy_engine::set_clock_for_test(Some(
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
            glib::MainContext::default().block_on(super::super::super::copy_engine::placed(
                placed.clone(),
                super::super::super::copy_engine::placed_budget(),
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

#[test]
fn an_undo_keeps_what_was_too_big_to_record() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // A replaced folder of four entries; a merge's two overwritten files,
    // which share the limit.
    for (merged, limit) in [(false, 3), (true, 1)] {
        let _limit = PlacedEntriesGuard::set(limit);
        let root = tempfile::tempdir()?;
        let target = root.path().join("target");
        if !merged {
            write_files(&target, &[("a.txt", "a"), ("sub/b.txt", "b")])?;
            let placed = placed_now(&[&target]);
            let original =
                staged_trash_folder(root.path(), "target", &target, &[("old.txt", "old")])?;
            let events = run_undo(
                (Vec::new(), vec![Location::local(&target)], placed),
                staged_lookup(HashMap::from([(target.clone(), original)])),
            );
            assert_eq!(names(&target)?, ["a.txt", "sub"]);
            assert_eq!(kept_in_trash(&events), changed_after_the_merge(&target));
            assert!(trash_in(root.path()).join("info/target.trashinfo").exists());
            continue;
        }
        let source = root.path().join("source");
        write_files(&source, &[("a.txt", "incoming a"), ("b.txt", "incoming b")])?;
        write_files(&target, &[("a.txt", "old a"), ("b.txt", "old b")])?;
        let (stage, _) = rename_stage(root.path().join("staging"));
        let (plans, on_merged) = recorded_plans();
        glib::MainContext::default().block_on(merge_local_with(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            false,
            gio::Cancellable::new(),
            None,
            MergeHooks {
                names: NamePolicy::default(),
                copy_into_target: real_copy(),
                stage_overwrite: stage,
                on_merged: &on_merged,
                progress: None,
            },
        ))?;
        let [a, b] = ["a.txt", "b.txt"].map(|name| target.join(name));
        let staged = HashMap::from([
            (
                a.clone(),
                staged_trash_fixture(root.path(), "a.txt", &a, b"old a")?,
            ),
            (
                b.clone(),
                staged_trash_fixture(root.path(), "b.txt", &b, b"old b")?,
            ),
        ]);
        let events = run_undo(undo_parts(plans.take()), staged_lookup(staged));
        assert_eq!(fs::read(&a)?, b"old a");
        assert_eq!(fs::read(&b)?, b"incoming b");
        assert_eq!(kept_in_trash(&events), changed_after_the_merge(&b));
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

        glib::MainContext::default().block_on(merge_local_with(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            false,
            gio::Cancellable::new(),
            None,
            MergeHooks {
                names: NamePolicy::default(),
                copy_into_target: real_copy(),
                stage_overwrite: stage,
                on_merged: &on_merged,
                progress: None,
            },
        ))?;

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

#[test]
fn a_merge_undo_without_trash_deletes_only_what_the_merge_created_unchanged()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // What happened to the created new.txt and sub after the merge, whether
    // undo has Trash, and whether the mount places folders or copies them in
    // place.
    let rows = [
        ("untouched", Trash::Unsupported, true),
        ("untouched", Trash::Unsupported, false),
        ("edited", Trash::Unsupported, true),
        ("added to", Trash::Unsupported, true),
        ("unrecorded", Trash::Unsupported, true),
        ("edited", Trash::Supported, true),
    ];
    for (change, trash, places_folders) in rows {
        let row = format!("{change}, {trash:?}, places folders: {places_folders}");
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
        let placement = if places_folders {
            Quirks::default()
        } else {
            Quirks {
                mount_type: Some(("ext4", "rw")),
                ..Quirks::without_folder_placement()
            }
        };
        let scope = FsQuirks::install(root.path(), Quirks { trash, ..placement });
        if !places_folders {
            learn_mount_capability(scope.key(), |capabilities| {
                capabilities.no_replace = Some(false);
                capabilities.hard_links = Some(true);
                capabilities.rename_over_empty_directory = Some(false);
            });
        }
        let (merged, plans) = merge_through_trash(&source, &target, false, real_copy());
        merged?;
        let [file, folder] = ["docs/new.txt", "docs/sub"].map(|path| target.join(path));
        let mut parts = undo_parts(plans);
        match change {
            "edited" => {
                fs::write(&file, b"edited after the merge")?;
                fs::write(folder.join("a.txt"), b"edited after the merge")?;
            }
            "added to" => fs::write(folder.join("c.txt"), b"added after the merge")?,
            "unrecorded" => parts.2.clear(),
            _ => {}
        }

        let events = run_undo(parts, staged_lookup(HashMap::new()));

        assert_eq!(
            fs::read(target.join("docs/stays.txt"))?,
            b"keep me",
            "{row}"
        );
        let mut trashed = trashed_items(&events)
            .into_iter()
            .map(|(location, _)| location)
            .collect::<Vec<_>>();
        trashed.sort_by_key(Location::display_name);
        let kept: &[&Path] = match (change, trash) {
            (_, Trash::Supported) => {
                assert_eq!(
                    trashed,
                    [Location::local(&file), Location::local(&folder)],
                    "{row}: Trash takes them whatever they hold"
                );
                &[]
            }
            ("untouched", _) => &[],
            ("added to", _) => &[&folder],
            _ => &[&file, &folder],
        };
        if !matches!(trash, Trash::Supported) {
            assert!(trashed.is_empty(), "{row}");
        }
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
            let mut expected = vec!["a.txt", "b.txt"];
            if change == "added to" {
                expected.push("c.txt");
            }
            assert_eq!(names(&folder)?, expected, "{row}: kept whole");
            assert_eq!(fs::read(folder.join("b.txt"))?, b"incoming b", "{row}");
        }
        if kept.is_empty() {
            assert!(
                matches!(events.last(), Some(OperationEvent::Restored { .. })),
                "{row}: {events:?}"
            );
            continue;
        }
        let Some(OperationEvent::CompletedWithErrors {
            deleted_locations,
            message,
            ..
        }) = events.last()
        else {
            panic!("{row}: {events:?}");
        };
        let deleted: Vec<_> = [&file, &folder]
            .into_iter()
            .filter(|item| !kept.contains(&item.as_path()))
            .map(Location::local)
            .collect();
        assert_eq!(*deleted_locations, deleted, "{row}");
        let reason = crate::i18n::tr("It was kept, as it may have changed since it was copied");
        for item in kept {
            let line =
                super::super::super::item_error(&Location::local(item).display_name(), &reason);
            assert!(message.contains(&line), "{row}: {message}");
        }
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
    for ((stage, published, tmpfile), change) in stages
        .into_iter()
        .flat_map(|stage| changes.map(|change| (stage, change)))
    {
        let context = format!("{change}, {stage}");
        let edited = change != "unchanged";
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
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
        let racing = change.ends_with("before recording");
        let mut after = Vec::new();
        if racing {
            for (op, name, relative) in [
                (published, "notes.txt", "notes.txt"),
                (Op::Rename, "plain", "plain/a.txt"),
            ] {
                let saved = root.path().join(format!("saved-{name}"));
                fs::write(&saved, b"concurrent arrival")?;
                let over = destination.join(relative);
                let in_place = change == "edited before recording";
                let racer: Racer = Box::new(move || {
                    if in_place {
                        fs::write(&over, b"concurrent arrival").expect("edit copied entry");
                    } else {
                        fs::rename(&saved, &over).expect("replace copied entry");
                    }
                });
                after.push((op, name, 0, racer));
            }
        }
        let _scope = FsQuirks::install(
            root.path(),
            without_trash(Quirks {
                after,
                tmpfile,
                ..Quirks::default()
            }),
        );
        let item = |name: &str, conflict| PasteItem {
            source: Location::local(sources.join(name)),
            conflict,
            target_name: None,
        };

        let events = run_paste(PasteRequest {
            id: OperationRequestId(1619),
            destination: Location::local(&destination),
            items: vec![
                item("notes.txt", TransferConflict::FailIfExists),
                item("docs", TransferConflict::Merge),
                item("keep.txt", TransferConflict::KeepBoth),
                item("plain", TransferConflict::FailIfExists),
            ],
            move_sources: false,
        });
        // As the app gathers the undo of a paste with a merge.
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
            assert_eq!(fs::read(&notes)?, b"concurrent arrival", "{context}");
            assert_eq!(fs::read(plain.join("a.txt"))?, b"concurrent arrival");
        }
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
        message.contains(&super::super::super::item_error("sub", &not_empty)),
        "{message}"
    );
    Ok(())
}

fn aside_folder(target: &Path) -> Result<Option<String>, io::Error> {
    Ok(names(target)?
        .into_iter()
        .find(|name| name.starts_with(".strata-merge-")))
}

fn merge_through_trash(
    source: &Path,
    target: &Path,
    move_source: bool,
    copy_into_target: CopyStep,
) -> (Result<(), glib::Error>, Vec<MergePlan>) {
    let (plans, on_merged) = recorded_plans();
    let result = glib::MainContext::default().block_on(merge_local_with(
        gio::File::for_path(source),
        gio::File::for_path(target),
        move_source,
        gio::Cancellable::new(),
        None,
        MergeHooks {
            names: NamePolicy::default(),
            copy_into_target,
            stage_overwrite: Rc::new(trash_stage_overwrite),
            on_merged: &on_merged,
            progress: None,
        },
    ));
    (result, plans.take())
}

#[test]
fn merge_without_trash_sets_originals_aside_and_removes_them_after_success()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (trash, quirks, move_source) in [
        (Trash::Unsupported, Quirks::default(), false),
        (Trash::Unsupported, Quirks::without_rename_flags(), true),
        (Trash::FalseWithoutError, Quirks::default(), false),
        (Trash::Unsupported, Quirks::sshfs(), false),
        (Trash::Unsupported, Quirks::vboxsf(), true),
        (Trash::Unsupported, Quirks::exfat(), false),
        (Trash::Unsupported, Quirks::exfat(), true),
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        write_files(
            &source,
            &[
                ("shared.txt", "new"),
                ("nested/deep/shared.txt", "new deep"),
                ("incoming.txt", "incoming"),
            ],
        )?;
        write_files(
            &target,
            &[
                ("shared.txt", "old"),
                ("nested/deep/shared.txt", "old deep"),
                ("nested/stays.txt", "keep me"),
            ],
        )?;
        let quirks = FsQuirks::install(root.path(), Quirks { trash, ..quirks });

        let (result, plans) = merge_through_trash(&source, &target, move_source, real_copy());

        assert!(result.is_ok(), "{trash:?}: {result:?}");
        assert!(quirks.used_name(".strata-merge-*"));
        assert_eq!(fs::read(target.join("shared.txt"))?, b"new");
        assert_eq!(
            fs::read(target.join("nested/deep/shared.txt"))?,
            b"new deep"
        );
        assert_eq!(fs::read(target.join("nested/stays.txt"))?, b"keep me");
        assert_eq!(names(&target)?, ["incoming.txt", "nested", "shared.txt"]);
        assert_eq!(source.exists(), !move_source);
        assert_eq!(
            plans[0].created,
            vec![Location::local(target.join("incoming.txt"))]
        );
        let (_, overwritten, placed) = undo_parts(plans);
        assert!(
            overwritten.is_empty(),
            "originals deleted without Trash cannot be restored by undo"
        );
        let recorded = if move_source {
            Vec::new()
        } else {
            vec![Location::local(target.join("incoming.txt"))]
        };
        assert_eq!(
            placed.into_keys().collect::<Vec<_>>(),
            recorded,
            "{trash:?}: only what undo deletes is recorded"
        );
    }
    Ok(())
}

#[test]
fn without_trash_an_original_the_merge_left_nothing_in_place_of_stays() -> Result<(), Box<dyn Error>>
{
    use super::super::fs_quirks::Racer;
    let _serial = serial()?;
    let rewrite = |path: PathBuf, contents: &'static [u8]| -> Racer {
        Box::new(move || fs::write(&path, contents).expect("rewrite the source"))
    };
    for row in [
        "left out",
        "kept at its source",
        "put back fails",
        "name taken meanwhile",
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::create_dir(&source)?;
        write_files(&target, &[("x", "original"), ("y", "kept")])?;
        let moving = row == "kept at its source";
        let mut source_quirks = Quirks::default();
        let _socket = if moving {
            fs::write(source.join("x"), b"incoming")?;
            source_quirks.before = vec![
                (Op::Read, "x", 0, rewrite(source.join("x"), b"second")),
                (Op::Read, "x", 1, rewrite(source.join("x"), b"third")),
            ];
            None
        } else {
            Some(std::os::unix::net::UnixListener::bind(source.join("x"))?)
        };
        let mut target_quirks = without_trash(Quirks::default());
        if row == "put back fails" {
            target_quirks.fail = vec![(Op::Rename, "x", 1, Errno::IO)];
        }
        if row == "name taken meanwhile" {
            target_quirks.before = vec![(Op::Rename, "x", 1, rewrite(target.join("x"), b"theirs"))];
        }
        let _source = FsQuirks::install(&source, source_quirks);
        let _target = FsQuirks::install(&target, target_quirks);

        let (result, _) = merge_through_trash(&source, &target, moving, real_copy());

        if matches!(row, "put back fails" | "name taken meanwhile") {
            let aside = aside_folder(&target)?.ok_or("the original stays aside")?;
            assert!(result.is_err(), "{row}");
            assert_eq!(
                fs::read(target.join(aside).join("x"))?,
                b"original",
                "{row}"
            );
            if row == "name taken meanwhile" {
                assert_eq!(fs::read(target.join("x"))?, b"theirs", "{row}");
            }
            continue;
        }
        assert!(result.is_ok(), "{row}: {result:?}");
        assert_eq!(names(&target)?, ["x", "y"], "{row}");
        assert_eq!(fs::read(target.join("x"))?, b"original", "{row}");
        if moving {
            assert_eq!(fs::read(source.join("x"))?, b"third", "{row}");
        }
    }
    Ok(())
}

#[test]
fn a_merge_without_trash_reports_its_folder_kept_by_an_original_still_open()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let editor = Some("Text Editor");
    let without_links = || Quirks {
        links: Support::Rejected(Errno::PERM),
        ..Quirks::nfs3()
    };
    let renumbered_links = || Quirks {
        link_inodes: true,
        ..Quirks::nfs3()
    };
    for (row, move_sources, original, preset, holder) in [
        ("copy fails", false, "x", Quirks::nfs3 as Preset, editor),
        ("copy fails", false, "sub/x", Quirks::nfs3, editor),
        ("copy fails, no links", false, "x", without_links, editor),
        (
            "copy fails, links numbered apart",
            false,
            "x",
            renumbered_links,
            editor,
        ),
        ("left out", false, "x", Quirks::nfs3, editor),
        ("left out", true, "sub/x", Quirks::nfs3, editor),
        ("replaced", true, "x", Quirks::nfs3, editor),
        ("replaced", true, "x", Quirks::cifs_posix, editor),
        ("replaced", true, "x", Quirks::cifs_posix, None),
        ("replaced", true, "sub/x", Quirks::nfs3, None),
        ("replaced", true, "sub/x", Quirks::cifs_posix, None),
    ] {
        let holder = holder.map(|program| crate::services::Holder::Program(program.to_owned()));
        let root = tempfile::tempdir()?;
        let linked_back = matches!(preset().links, Support::Supported)
            && !preset().link_inodes
            && base_links_keep_their_number(root.path())?;
        let put_back_by_rename = !row.starts_with("replaced") && !linked_back;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        let (source, target) = (sources.join("run"), destination.join("run"));
        write_files(&source, &[("y.txt", "new y")])?;
        write_files(&target, &[(original, "original")])?;
        if row == "replaced" {
            write_files(&source, &[("also/z", "incoming")])?;
            write_files(&target, &[("also/z", "original")])?;
        }
        fs::create_dir_all(source.join(original).parent().ok_or("no parent")?)?;
        let mut source_quirks = Quirks::default();
        let _socket = if row == "left out" {
            match std::os::unix::net::UnixListener::bind(source.join(original)) {
                Ok(socket) => Some(socket),
                Err(error)
                    if matches!(Errno::from_io_error(&error), Some(Errno::PERM | Errno::IO)) =>
                {
                    eprintln!(
                        "skipped on a base that refuses Unix sockets: {row} {original}, moving: {move_sources}"
                    );
                    continue;
                }
                Err(error) => return Err(error.into()),
            }
        } else {
            fs::write(source.join(original), b"incoming")?;
            None
        };
        if row.starts_with("copy fails") {
            source_quirks.fail = vec![(Op::OpenSource, "x", 0, Errno::IO)];
        }
        let _sources = FsQuirks::install(&sources, source_quirks);
        let _destination = FsQuirks::install(
            &destination,
            without_trash(Quirks {
                holder: holder.clone(),
                ..preset()
            }),
        );
        let _clock = ClockGuard::set(std::time::SystemTime::now());
        let _held = hold(&target.join(original));
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let _operation = LocalOperationProvider.paste(
            PasteRequest {
                id: OperationRequestId(1729),
                destination: Location::local(&destination),
                items: vec![PasteItem {
                    source: Location::local(&source),
                    conflict: TransferConflict::Merge,
                    target_name: None,
                }],
                move_sources,
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );

        drive_until_transfer_settles(&events);

        let listed = !matches!(
            preset().silly_rename,
            Some(super::super::fs_quirks::Silly {
                aside: super::super::fs_quirks::Aside::Unlisted(_),
                ..
            })
        );
        let row = format!(
            "{row} {original}, moving: {move_sources}, {:?}",
            preset().mount_type
        );
        let events = events.take();
        let folder = aside_folder(&target)?;
        assert_eq!(folder.is_none(), put_back_by_rename, "{row}: {events:?}");
        let kept = events
            .iter()
            .filter_map(|event| match event {
                OperationEvent::ItemsReported { report, .. } => Some(report.kept.clone()),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        let cause = match holder {
            None if !listed => KeptCause::Undeletable(Errno::ACCESS.raw_os_error()),
            holder => KeptCause::StillOpen(holder),
        };
        let mut expected: Vec<_> = folder
            .as_ref()
            .map(|folder| KeptItem {
                location: Location::local(target.join(folder)),
                cause,
            })
            .into_iter()
            .collect();
        if row.starts_with("left out") && move_sources {
            expected.push(KeptItem {
                location: Location::local(source.join(original)),
                cause: KeptCause::Skipped(crate::services::SpecialKind::Socket),
            });
        }
        assert_eq!(kept, expected, "{row}: {events:?}");
        let now_there: &[u8] = if row.starts_with("replaced") {
            b"incoming"
        } else {
            b"original"
        };
        assert_eq!(fs::read(target.join(original))?, now_there, "{row}");
        if let Some(folder) = &folder {
            let aside = target.join(folder).join(original);
            let leftovers = names(aside.parent().ok_or("no parent")?)?;
            assert!(
                leftovers.len() == usize::from(listed)
                    && leftovers.iter().all(|name| name.starts_with(".nfs")),
                "{row}: {leftovers:?}"
            );
        }
        match terminal_transfer(&events) {
            Some(OperationEvent::TransferFailed { message, .. }) if row.starts_with("copy") => {
                assert!(
                    folder.iter().all(|folder| !message.contains(folder)),
                    "{row}: {message}"
                );
                assert_eq!(fs::read(source.join(original))?, b"incoming", "{row}");
            }
            Some(OperationEvent::Pasted { .. }) if !row.starts_with("copy") => {
                assert_eq!(fs::read(target.join("y.txt"))?, b"new y", "{row}");
                for name in ["y.txt", original] {
                    let gone = move_sources && (name == "y.txt" || row.starts_with("replaced"));
                    assert_eq!(source.join(name).exists(), !gone, "{row}: {name}");
                }
                assert_eq!(
                    item_moves(&events).len(),
                    usize::from(move_sources),
                    "{row}"
                );
            }
            other => panic!("{row}: {other:?}"),
        }
    }
    Ok(())
}

#[test]
fn a_failed_merge_without_trash_puts_back_what_it_can_and_names_the_rest()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (failure, reason) in [
        (gio::IOErrorEnum::Failed, "injected copy failure"),
        (gio::IOErrorEnum::Cancelled, "already exists"),
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        write_files(&source, &[("a.txt", "new a"), ("b.txt", "new b")])?;
        write_files(
            &target,
            &[
                ("a.txt", "old a"),
                ("b.txt", "old b"),
                ("stays.txt", "keep me"),
            ],
        )?;
        let _quirks = FsQuirks::install(root.path(), without_trash(Quirks::default()));
        let partial: CopyStep = Rc::new(move |_, target, _, _| {
            Box::pin(async move {
                let path = target
                    .path()
                    .ok_or_else(|| io_error("missing target path"))?;
                fs::write(path.join("a.txt"), b"partial").map_err(io_error)?;
                Err(glib::Error::new(failure, "injected copy failure"))
            })
        });

        let (result, plans) = merge_through_trash(&source, &target, false, partial);

        let error = result.expect_err("the copy failed");
        let folder = aside_folder(&target)?.ok_or("the displaced original was not kept")?;
        assert!(error.message().contains(&folder), "{error}");
        assert!(error.message().contains(reason), "{error}");
        assert_eq!(fs::read(target.join(&folder).join("a.txt"))?, b"old a");
        assert_eq!(names(&target.join(&folder))?, ["a.txt"]);
        assert_eq!(fs::read(target.join("a.txt"))?, b"partial");
        assert_eq!(fs::read(target.join("b.txt"))?, b"old b");
        assert_eq!(fs::read(target.join("stays.txt"))?, b"keep me");
        assert!(plans.iter().all(|plan| plan.overwritten.is_empty()));
    }
    Ok(())
}

#[test]
fn a_failed_aside_puts_every_original_back_before_merging() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let target_files = [
        ("a.txt", "old a"),
        ("b.txt", "old b"),
        ("mount/c.txt", "old c"),
    ];
    for case in [
        "no safe placement",
        "second aside fails",
        "nested mount",
        "strict open crosses mount",
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        write_files(
            &source,
            &[
                ("a.txt", "new a"),
                ("b.txt", "new b"),
                ("mount/c.txt", "new c"),
            ],
        )?;
        write_files(&target, &target_files)?;
        let quirks = match case {
            "no safe placement" => Quirks::without_placement(),
            "second aside fails" => Quirks {
                fail: vec![(Op::Rename, "*", 1, Errno::IO)],
                ..Quirks::default()
            },
            "strict open crosses mount" => Quirks {
                fail: vec![(Op::OpenAt, ".strata-merge-*", 0, Errno::XDEV)],
                ..Quirks::default()
            },
            _ => Quirks::default(),
        };
        let quirks = FsQuirks::install(root.path(), without_trash(quirks));
        if case == "second aside fails" {
            let folder = fs::File::open(root.path())?;
            let no_replace = probe(&System, folder.as_fd(), true, false, false)?.no_replace;
            learn_mount_capability(quirks.key(), |capabilities| {
                capabilities.no_replace = no_replace;
            });
        }
        let _nested = (case == "nested mount")
            .then(|| FsQuirks::install(&target.join("mount"), without_trash(Quirks::default())));
        let copied = Rc::new(Cell::new(false));
        let copy_observed = copied.clone();

        let (result, plans) = merge_through_trash(
            &source,
            &target,
            false,
            Rc::new(move |_, _, _, _| {
                copy_observed.set(true);
                Box::pin(async { Ok(MergeCopied::default()) })
            }),
        );

        let error = result.expect_err(case);
        assert!(
            error.message().contains("out of the way"),
            "{case}: {error}"
        );
        assert!(!copied.get(), "{case}");
        assert_eq!(
            quirks.used_name(".strata-merge-*"),
            case != "no safe placement",
            "{case}"
        );
        for (path, contents) in target_files {
            assert_eq!(fs::read(target.join(path))?, contents.as_bytes(), "{case}");
        }
        assert_eq!(names(&target)?, ["a.txt", "b.txt", "mount"], "{case}");
        assert!(plans.iter().all(|plan| plan.overwritten.is_empty()));
    }
    Ok(())
}

#[test]
fn a_flushed_merge_move_keeps_sources_when_published_entries_disappear()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for missing in ["", "nested", "nested/a.txt", "nested/a (1).txt"] {
        for replaced in [false, true] {
            let root = tempfile::tempdir()?;
            let source = root.path().join("sources/target");
            let destination = root.path().join("destination");
            let target = destination.join("target");
            write_files(&source, &[("nested/a.txt", "incoming")])?;
            fs::create_dir_all(target.join("nested"))?;
            let mut before = Vec::new();
            if missing.ends_with("(1).txt") {
                let taken = target.join("nested/a.txt");
                let racer: Racer =
                    Box::new(move || fs::write(&taken, b"arrival").expect("take name"));
                before.push((Op::Rename, "a.txt", 0, racer));
            }
            let gone = target.join(missing);
            let racer: Racer = Box::new(move || {
                if gone.is_dir() {
                    fs::remove_dir_all(&gone).expect("remove copied directory");
                    if replaced {
                        fs::create_dir(&gone).expect("replace directory");
                    }
                } else {
                    fs::remove_file(&gone).expect("remove copied file");
                    if replaced {
                        fs::write(&gone, b"replacement").expect("replace file");
                    }
                }
            });
            before.push((Op::SyncFs, "*", 0, racer));
            let _quirks = FsQuirks::install(
                root.path(),
                without_trash(Quirks {
                    before,
                    ..Quirks::default()
                }),
            );
            let events = run_paste(PasteRequest {
                id: OperationRequestId(1619),
                destination: Location::local(&destination),
                items: vec![PasteItem {
                    source: Location::local(&source),
                    conflict: TransferConflict::Merge,
                    target_name: None,
                }],
                move_sources: true,
            });
            assert!(
                matches!(events.last(), Some(OperationEvent::Pasted { .. })),
                "{missing}, replaced {replaced}: {events:?}"
            );
            let kept = events
                .iter()
                .filter_map(|event| match event {
                    OperationEvent::ItemsReported { report, .. } => Some(report.kept.clone()),
                    _ => None,
                })
                .flatten()
                .collect::<Vec<_>>();
            assert_eq!(
                kept,
                [KeptItem {
                    location: Location::local(&source),
                    cause: KeptCause::CopyGone
                }],
                "{missing}, replaced {replaced}"
            );
            assert_eq!(
                fs::read(source.join("nested/a.txt"))?,
                b"incoming",
                "{missing}, replaced {replaced}"
            );
        }
    }
    Ok(())
}

#[test]
fn merge_originals_are_deleted_only_once_the_merge_is_flushed() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for flushed in [true, false] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        write_files(&source, &[("shared.txt", "new")])?;
        write_files(&target, &[("shared.txt", "old")])?;
        let aside_during_flush = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (record, watched) = (aside_during_flush.clone(), target.clone());
        let racer: super::super::fs_quirks::Racer = Box::new(move || {
            let kept = aside_folder(&watched).is_ok_and(|folder| folder.is_some());
            record
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(kept);
        });
        let _quirks = FsQuirks::install(
            root.path(),
            without_trash(Quirks {
                before: vec![(Op::SyncFs, "*", 0, racer)],
                fail: if flushed {
                    Vec::new()
                } else {
                    vec![(Op::SyncFs, "*", 0, Errno::IO)]
                },
                ..Quirks::default()
            }),
        );

        let (result, _) = merge_through_trash(&source, &target, true, real_copy());

        assert_eq!(
            *aside_during_flush
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
            [true],
            "one flush before the originals are deleted, shared with the move"
        );
        assert_eq!(fs::read(target.join("shared.txt"))?, b"new");
        if flushed {
            assert!(result.is_ok(), "{result:?}");
            assert_eq!(names(&target)?, ["shared.txt"]);
            assert!(!source.exists());
            continue;
        }
        let aside = aside_folder(&target)?.ok_or("the originals stay aside")?;
        assert_eq!(fs::read(target.join(&aside).join("shared.txt"))?, b"old");
        assert_eq!(fs::read(source.join("shared.txt"))?, b"new");
        let error = result.expect_err("the originals and the source are kept");
        assert!(
            error.message().contains(&aside)
                && error.message().contains("writes may still be pending"),
            "{error}"
        );
    }
    Ok(())
}

/// Lists local folders, watching them only when `watched`, as where no
/// folder monitor is available.
struct Listing {
    watched: bool,
}

impl crate::services::FileSource for Listing {
    fn validate_location(
        &self,
        location: &Location,
    ) -> Result<(), crate::services::LocationValidationError> {
        crate::adapters::LocalFileSource.validate_location(location)
    }

    fn validate_location_async(
        &self,
        location: Location,
        emit: Rc<dyn Fn(Result<(), crate::services::LocationValidationError>)>,
    ) -> crate::services::LoadHandle {
        crate::adapters::LocalFileSource.validate_location_async(location, emit)
    }

    fn enumerate(
        &self,
        request: crate::services::DirectoryRequest,
        emit: Rc<dyn Fn(crate::services::DirectoryEvent)>,
    ) -> crate::services::LoadHandle {
        crate::adapters::LocalFileSource.enumerate(request, emit)
    }

    fn start_watch(
        &self,
        location: Location,
        include_hidden: bool,
        notify: Rc<dyn Fn(crate::services::DirectoryChange)>,
        report: Rc<dyn Fn(crate::services::Watching)>,
    ) -> crate::services::LoadHandle {
        if self.watched {
            crate::adapters::LocalFileSource.start_watch(location, include_hidden, notify, report)
        } else {
            report(crate::services::Watching::Unavailable);
            crate::services::LoadHandle::new(|| {})
        }
    }
}

#[test]
fn a_merge_undo_drops_from_view_only_what_it_removed() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::merge::a_merge_undo_drops_from_view_only_what_it_removed",
        || {
            check_a_merge_undo_drops_from_view_only_what_it_removed().expect("merge undo view");
        },
    );
}

fn check_a_merge_undo_drops_from_view_only_what_it_removed() -> Result<(), Box<dyn Error>> {
    use crate::app::{Browser, BrowserEvent};
    let _serial = serial()?;
    let context = glib::MainContext::default();
    let pump_until = |what: &str, done: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            if Instant::now() > deadline {
                return Err(format!("timed out waiting until {what}"));
            }
            context.iteration(false);
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    };
    let sandbox = glib::user_data_dir()
        .parent()
        .ok_or("the test's data folder")?
        .to_path_buf();
    let mut failures = Vec::new();
    let rows = [false, true].into_iter().flat_map(|failing| {
        [(false, false), (true, false), (false, true), (true, true)]
            .map(|(background, watched)| (failing, background, watched))
    });
    // Without a monitor, the folder learns only from the undo's outcome.
    for (failing, background, watched) in rows {
        let row = format!(
            "undo fails an item: {failing}, merged in the background: {background}, watched: {watched}"
        );
        let root = tempfile::tempdir_in(glib::home_dir())?;
        let source = root.path().join("source");
        let folder = root.path().join("destination/folder");
        write_files(
            &source,
            &[
                ("folder/a.txt", "incoming a"),
                ("folder/b.txt", "incoming b"),
                ("folder/c.txt", "incoming c"),
                ("folder/d.txt", "incoming d"),
                ("folder/new.txt", "new"),
            ],
        )?;
        write_files(
            &folder,
            &[
                ("a.txt", "original a"),
                ("b.txt", "original b"),
                ("c.txt", "original c"),
                ("d.txt", "original d"),
            ],
        )?;
        // The Trash shares the scope, so it is on the folder's mount.
        let _scope = FsQuirks::install(
            &sandbox,
            Quirks {
                fail: if failing {
                    vec![(Op::Unlink, "c.txt", 0, Errno::ACCESS)]
                } else {
                    Vec::new()
                },
                ..Quirks::default()
            },
        );
        let browser = Browser::new(Rc::new(Listing { watched }));
        browser.set_operation_provider(Rc::new(LocalOperationProvider));
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        browser.observe(move |event| {
            observed
                .borrow_mut()
                .push(crate::test_support::operations::operation_event(event).clone());
        });
        let names = || {
            let mut names = browser
                .with_column_entries(0, |entries| {
                    entries
                        .iter()
                        .map(|entry| entry.display_name.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            names.sort();
            names
        };

        browser.transfer(
            Location::local(root.path().join("destination")),
            vec![PasteItem {
                source: Location::local(source.join("folder")),
                conflict: TransferConflict::Merge,
                target_name: None,
            }],
            false,
            false,
        );
        if background {
            let id = browser.last_started_operation().ok_or("the merge")?;
            assert!(browser.background_file_operation(id), "{row}");
        }
        pump_until(&format!("{row}: the merge ends"), &|| {
            events
                .borrow()
                .iter()
                .any(|event| matches!(event, BrowserEvent::TransferCompleted))
        })?;
        fs::write(folder.join("b.txt"), "edited after the merge")?;
        fs::remove_file(folder.join("d.txt"))?;
        browser.navigate(Location::local(&folder));
        pump_until(&format!("{row}: the folder lists the merge"), &|| {
            names() == ["a.txt", "b.txt", "c.txt", "new.txt"]
        })?;
        let (generation, created, overwritten) =
            browser.pending_undo_merge().ok_or("the merge's undo")?;
        events.borrow_mut().clear();

        assert!(
            browser.undo_merge(generation, created, overwritten),
            "{row}"
        );
        pump_until(&format!("{row}: the undo ends"), &|| {
            events
                .borrow()
                .iter()
                .any(|event| matches!(event, BrowserEvent::RestorationFinished { .. }))
        })?;

        assert_eq!(
            events
                .borrow()
                .iter()
                .any(|event| matches!(event, BrowserEvent::OperationCompletedWithErrors { .. })),
            failing,
            "{row}"
        );
        assert_eq!(fs::read(folder.join("a.txt"))?, b"original a", "{row}");
        assert_eq!(
            fs::read(folder.join("b.txt"))?,
            b"edited after the merge",
            "{row}"
        );
        assert_eq!(
            fs::read(folder.join("c.txt"))?,
            if failing {
                &b"incoming c"[..]
            } else {
                b"original c"
            },
            "{row}"
        );
        assert_eq!(fs::read(folder.join("d.txt"))?, b"original d", "{row}");
        assert!(!folder.join("new.txt").exists(), "{row}");
        let expected = ["a.txt", "b.txt", "c.txt", "d.txt"];
        // A monitor may still be reporting what the undo changed, and a
        // folder without one reloads.
        if pump_until(&row, &|| names() == expected).is_err() {
            failures.push(format!("{row}: the folder lists {:?}", names()));
        }
        assert_eq!(
            browser
                .pending_undo_merge()
                .map(|(_, created, overwritten)| (created, overwritten)),
            failing.then(|| (Vec::new(), vec![Location::local(folder.join("c.txt"))])),
            "{row}: only what failed is left to undo"
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn an_undo_never_brings_back_another_item_trashed_later_from_the_same_path() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::merge::an_undo_never_brings_back_another_item_trashed_later_from_the_same_path",
        || {
            for undone in [Undone::Delete, Undone::Replace, Undone::Merge] {
                check_an_undo_never_brings_back_another_item_trashed_later(undone)
                    .unwrap_or_else(|error| panic!("{undone:?}: {error}"));
            }
        },
    );
}

fn check_an_undo_never_brings_back_another_item_trashed_later(
    undone: Undone,
) -> Result<(), Box<dyn Error>> {
    use crate::app::{Browser, BrowserEvent};
    let _serial = serial()?;
    let context = glib::MainContext::default();
    let root = tempfile::tempdir_in(glib::home_dir())?;
    let target = root.path().join("folder/report.txt");
    write_files(root.path(), &[("folder/report.txt", "original")])?;
    let browser = Browser::new(Rc::new(Listing { watched: false }));
    browser.set_operation_provider(Rc::new(LocalOperationProvider));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| {
        observed
            .borrow_mut()
            .push(crate::test_support::operations::operation_event(event).clone());
    });
    let pump_until = |what: &str, done: &dyn Fn(&BrowserEvent) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !events.borrow().iter().any(done) {
            if Instant::now() > deadline {
                return Err(format!("timed out waiting until {what}"));
            }
            context.iteration(false);
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    };
    let incoming = |conflict| -> Result<(), Box<dyn Error>> {
        write_files(
            &root.path().join("incoming"),
            &[("folder/report.txt", "incoming")],
        )?;
        let (source, destination) = match conflict {
            TransferConflict::Merge => {
                (root.path().join("incoming/folder"), root.path().to_owned())
            }
            _ => (
                root.path().join("incoming/folder/report.txt"),
                root.path().join("folder"),
            ),
        };
        browser.transfer(
            Location::local(destination),
            vec![PasteItem {
                source: Location::local(source),
                conflict,
                target_name: None,
            }],
            false,
            false,
        );
        pump_until("the paste ends", &|event| {
            matches!(event, BrowserEvent::TransferCompleted)
        })?;
        Ok(())
    };
    match undone {
        Undone::Delete => {
            browser.delete(vec![file_entry(&target)], false);
            pump_until("the delete ends", &|event| {
                matches!(event, BrowserEvent::DeletionFinished { .. })
            })?;
        }
        Undone::Replace => incoming(TransferConflict::ReplaceExisting)?,
        Undone::Merge => incoming(TransferConflict::Merge)?,
    }
    let trash = glib::user_data_dir().join("Trash");
    let recorded = |info: &Path| {
        fs::read_to_string(info).ok().and_then(|contents| {
            contents
                .lines()
                .find_map(|line| line.strip_prefix("Path="))
                .and_then(crate::adapters::trash_restore::decode_trashinfo_path)
        })
    };
    let ours = fs::read_dir(trash.join("info"))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|info| recorded(info).as_deref() == Some(target.as_path()))
        .ok_or("the original's Trash entry")?;
    let contents = fs::read_to_string(&ours)?;
    fs::write(
        &ours,
        contents
            .lines()
            .map(|line| match line.strip_prefix("DeletionDate=") {
                Some(_) => "DeletionDate=2000-01-01T00:00:00",
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )?;
    let later = trash.join("files/report.later.txt");
    fs::write(&later, "trashed later")?;
    fs::write(
        trash.join("info/report.later.txt.trashinfo"),
        format!(
            "[Trash Info]\nPath={}\nDeletionDate=2099-01-01T00:00:00\n",
            target.display()
        ),
    )?;
    events.borrow_mut().clear();

    let started = match undone {
        Undone::Delete => browser.undo_last_trash(),
        Undone::Replace | Undone::Merge => {
            let (generation, created, overwritten) =
                browser.pending_undo_merge().ok_or("the paste's undo")?;
            browser.undo_merge(generation, created, overwritten)
        }
    };
    assert!(started, "the undo starts");
    pump_until("the undo ends", &|event| {
        matches!(event, BrowserEvent::RestorationFinished { .. })
    })?;

    assert_eq!(fs::read_to_string(&target)?, "original");
    assert_eq!(fs::read_to_string(&later)?, "trashed later");
    fs::remove_file(later)?;
    fs::remove_file(trash.join("info/report.later.txt.trashinfo"))?;
    Ok(())
}
