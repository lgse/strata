// SPDX-License-Identifier: MIT

use super::*;

fn cancels(cancellable: &gio::Cancellable) -> Racer {
    let cancel = cancellable.clone();
    Box::new(move || cancel.cancel())
}

fn locations(paths: &[&Path]) -> Vec<Location> {
    paths.iter().map(Location::local).collect()
}

#[test]
fn a_cancelled_merge_undo_reports_everything_not_attempted() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let target = root.path().join("target");
    fs::create_dir_all(&target)?;
    let shared = target.join("shared.txt");
    let incoming = target.join("incoming.txt");
    fs::write(&shared, b"incoming wins")?;
    fs::write(&incoming, b"new")?;
    let cancellable = gio::Cancellable::new();
    cancellable.cancel();

    let parts = (
        vec![Location::local(&incoming)],
        vec![Location::local(&shared)],
        HashMap::new(),
    );
    let events = undo_merge(93, parts, cancellable, staged_lookup(HashMap::new()));

    let [completed, _, not_attempted] = cancel_outcome(&events, "");
    assert!(completed.is_empty());
    assert_eq!(not_attempted, locations(&[&shared, &incoming]));
    assert_files(
        &target,
        &[("shared.txt", "incoming wins"), ("incoming.txt", "new")],
        "",
    )?;
    Ok(())
}

/// Two merged files whose undo is cut off by a cancel at `check` of the
/// first: none is Trash's, then the comparison's and the delete's.
fn check_cut_off_undo(row: &str, check: Option<usize>) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let created = ["first.txt", "second.txt"].map(|name| root.path().join(name));
    for item in &created {
        fs::write(item, b"merged")?;
    }
    let placed = placed_now(&[&created[0], &created[1]]);
    let cancellable = gio::Cancellable::new();
    let cancel = cancels(&cancellable);
    let quirks = match check {
        None => Quirks {
            before: vec![(Op::Trash, "first.txt", 0, cancel)],
            ..Quirks::default()
        },
        Some(check) => without_trash(Quirks {
            after: vec![(Op::Stat, "first.txt", check, cancel)],
            ..Quirks::default()
        }),
    };
    let _scope = FsQuirks::install(root.path(), quirks);

    let parts = (
        created.iter().map(Location::local).collect(),
        Vec::new(),
        placed,
    );
    let events = undo_merge(1615, parts, cancellable, staged_lookup(HashMap::new()));

    let [first, second] = &created;
    let [completed, failed, not_attempted] = cancel_outcome(&events, row);
    assert!(completed.is_empty(), "{row}");
    if row == "delete" {
        assert!(!first.exists(), "the cancel landed before the delete");
        assert_eq!(
            (failed, not_attempted),
            (locations(&[first]), locations(&[second]))
        );
    } else {
        assert!(failed.is_empty(), "{row}");
        assert_eq!(not_attempted, locations(&[first, second]), "{row}");
        assert_eq!(fs::read(first)?, b"merged", "{row}");
    }
    assert_eq!(fs::read(second)?, b"merged", "{row}");
    Ok(())
}

#[test]
fn a_merge_undo_cut_off_counts_an_item_as_not_attempted_only_when_it_left_it_whole()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // Without Trash, the first check is Trash's, the second the comparison's
    // and the third the delete's.
    for (row, check) in [("Trash", None), ("compare", Some(1)), ("delete", Some(2))] {
        check_cut_off_undo(row, check)?;
    }
    Ok(())
}

#[test]
fn a_kept_item_counts_as_failed_when_a_later_item_is_cut_off() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let [kept, cut] = ["kept.txt", "cut.txt"].map(|name| root.path().join(name));
    for item in [&kept, &cut] {
        fs::write(item, b"merged")?;
    }
    let placed = placed_now(&[&cut]);
    let cancellable = gio::Cancellable::new();
    let _scope = FsQuirks::install(
        root.path(),
        without_trash(Quirks {
            after: vec![(Op::Stat, "cut.txt", 1, cancels(&cancellable))],
            ..Quirks::default()
        }),
    );

    let parts = (locations(&[&kept, &cut]), Vec::new(), placed);
    let events = undo_merge(1617, parts, cancellable, staged_lookup(HashMap::new()));

    let outcome = cancel_outcome(&events, "");
    assert_eq!(
        outcome,
        [Vec::new(), locations(&[&kept]), locations(&[&cut])],
        "{outcome:?}"
    );
    assert_eq!(kept_items(&events), [Location::local(&kept)]);
    assert_files(
        root.path(),
        &[("kept.txt", "merged"), ("cut.txt", "merged")],
        "",
    )?;
    Ok(())
}

#[test]
fn a_merge_undo_never_counts_an_item_it_could_not_check_as_undone() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let [first, second] = ["first.txt", "second.txt"].map(|name| root.path().join(name));
    for item in [&first, &second] {
        fs::write(item, b"merged")?;
    }
    let unreadable = Location::uri("strata-test-no-backend:///odd.txt");
    let placed = placed_now(&[&first, &second]);
    let _scope = FsQuirks::install(root.path(), without_trash(Quirks::default()));

    // The cancel runs ahead of the reply to the next item's check, which is
    // then in flight.
    let parts = (
        vec![
            Location::local(&first),
            unreadable.clone(),
            Location::local(&second),
        ],
        Vec::new(),
        placed,
    );
    let lookup = staged_lookup(HashMap::new());
    let events = undo_merge_cancelling(1615, parts, (gio::Cancellable::new(), lookup), |event| {
        matches!(event, OperationEvent::DeleteProgress { completed: 2, .. })
    });

    assert_eq!(
        cancel_outcome(&events, ""),
        [
            locations(&[&first]),
            vec![unreadable],
            locations(&[&second])
        ]
    );
    assert!(!first.exists());
    assert_eq!(fs::read(&second)?, b"merged");
    Ok(())
}

/// Whether the merge's copy is still at the last path, which the undo
/// deletes before it restores the original, and whether the cut-off copy
/// of the original cannot be removed.
fn check_restore_cut_off(incoming: bool, stuck: bool) -> Result<(), Box<dyn Error>> {
    let row = format!("{incoming}, {stuck}");
    let root = tempfile::tempdir()?;
    let [first, second] = ["first.bin", "second.bin"].map(|name| root.path().join(name));
    let original = vec![0x5a; 4 * 1024 * 1024];
    fs::write(&first, b"incoming")?;
    if incoming {
        fs::write(&second, b"incoming")?;
    }
    let staged = HashMap::from([
        (
            first.clone(),
            staged_trash_fixture(root.path(), "first.bin", &first, b"first original")?,
        ),
        (
            second.clone(),
            staged_trash_fixture(root.path(), "second.bin", &second, &original)?,
        ),
    ]);
    let trash = trash_in(root.path());
    let cancellable = gio::Cancellable::new();
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![
                (Op::CopyRange, "second.bin", 0, cancels(&cancellable)),
                (Op::Write, "second.bin", 0, cancels(&cancellable)),
            ],
            fail: if stuck {
                vec![(Op::Unlink, "second.bin", 0, Errno::IO)]
            } else {
                Vec::new()
            },
            reflink: Support::Rejected(Errno::OPNOTSUPP),
            ..Quirks::without_placement()
        },
    );

    let placed = placed_now(&[&first, &second]);
    let parts = (Vec::new(), locations(&[&first, &second]), placed);
    let events = undo_merge(1615, parts, cancellable, staged_lookup(staged));

    let cut_off = locations(&[&second]);
    let (failed, not_attempted) = if incoming || stuck {
        (cut_off, Vec::new())
    } else {
        (Vec::new(), cut_off)
    };
    assert_eq!(
        cancel_outcome(&events, &row),
        [locations(&[&first]), failed, not_attempted],
        "{row}"
    );
    assert_eq!(fs::read(&first)?, b"first original");
    let left = fs::symlink_metadata(&second).is_ok();
    assert_eq!(left, stuck, "{row}");
    assert_eq!(fs::read(trash.join("files/second.bin"))?, original);
    assert!(trash.join("info/second.bin.trashinfo").exists());
    Ok(())
}

#[test]
fn a_merge_undo_cancelled_while_it_restores_an_original_ends_as_a_cancel()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (incoming, stuck) in [(true, false), (false, false), (false, true)] {
        check_restore_cut_off(incoming, stuck)?;
    }
    Ok(())
}

const UNREADABLE: &str = "strata-test-no-backend:///second.txt";

/// Hands out each staged entry once, by the location it stands for.
fn staged_once(entries: HashMap<Location, RestoreEntry>) -> StagedOriginalLookup {
    let entries = RefCell::new(entries);
    Rc::new(move |location, _| {
        let entry = entries.borrow_mut().remove(&location);
        Box::pin(async move { entry.ok_or_else(|| io_error("not staged")) })
    })
}

fn check_path_checked(case: &str) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let [first, second] = ["first.txt", "second.txt"].map(|name| root.path().join(name));
    fs::write(&first, b"merged")?;
    if case == "cancelled" {
        fs::write(&second, b"merged")?;
    }
    let target = match case {
        "failed" => Location::uri(UNREADABLE),
        _ => Location::local(&second),
    };
    let lookup = staged_once(HashMap::from([
        (
            Location::local(&first),
            staged_trash_fixture(root.path(), "first.txt", &first, b"first")?,
        ),
        (
            target.clone(),
            staged_trash_fixture(root.path(), "second.txt", &second, b"second")?,
        ),
    ]));
    let trash = trash_in(root.path());

    // A cancel runs ahead of the reply to the second path's check, then in flight.
    let cancelling = case == "cancelled";
    let placed = placed_now(&[&first]);
    let parts = (Vec::new(), vec![Location::local(&first), target], placed);
    let events = undo_merge_cancelling(
        1615,
        parts,
        (gio::Cancellable::new(), lookup),
        move |event| {
            cancelling && matches!(event, OperationEvent::RestoreProgress { completed: 1, .. })
        },
    );

    assert_eq!(fs::read(&first)?, b"first", "{case}");
    let staged_second = fs::read(trash.join("files/second.txt"));
    match (case, events.last()) {
        ("failed", Some(OperationEvent::CompletedWithErrors { message, .. })) => {
            assert_unreadable_left_staged(message, &trash, &second)?;
            assert_eq!(staged_second?, b"second");
        }
        ("cancelled", Some(OperationEvent::Cancelled { result, .. })) => {
            assert_eq!(
                [&result.completed, &result.failed, &result.not_attempted],
                [&locations(&[&first]), &Vec::new(), &locations(&[&second])]
            );
            assert_eq!(staged_second?, b"second");
            assert_eq!(fs::read(&second)?, b"merged");
        }
        ("missing", Some(OperationEvent::Restored { .. })) => {
            assert_eq!(fs::read(&second)?, b"second");
            assert!(staged_second.is_err());
        }
        (case, last) => panic!("{case}: {last:?}"),
    }
    Ok(())
}

/// The undo names why the unreadable path failed its check, and leaves the
/// second original staged.
fn assert_unreadable_left_staged(message: &str, trash: &Path, second: &Path) -> io::Result<()> {
    let check = gio::File::for_uri(UNREADABLE)
        .query_info(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            gio::Cancellable::NONE,
        )
        .expect_err("no backend");
    assert!(
        message.contains(&crate::services::gio_error_detail(&check)),
        "{message}"
    );
    assert!(trash.join("info/second.txt.trashinfo").exists());
    assert!(fs::symlink_metadata(second).is_err());
    Ok(())
}

#[test]
fn a_merge_undo_restores_an_original_only_over_a_path_it_checked() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // The second path's check fails, a cancel cuts it off, or the merge's
    // copy is already gone from it.
    for case in ["failed", "cancelled", "missing"] {
        check_path_checked(case)?;
    }
    Ok(())
}
