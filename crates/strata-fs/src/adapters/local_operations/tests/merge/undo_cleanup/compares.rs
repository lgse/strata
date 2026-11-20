// SPDX-License-Identifier: MIT

use super::*;

/// Merges incoming `docs/a.txt` and `docs/b.txt` over originals beside a
/// file that stays, linking the incoming pair where `change` says.
fn merge_docs(root: &Path, change: &str) -> Result<(PathBuf, UndoParts), Box<dyn Error>> {
    let source = root.join("source");
    let target = root.join("target");
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
    let (stage, _) = rename_stage(root.join("staging"));
    let (plans, on_merged) = recorded_plans();
    merge_staged(&source, &target, stage, &on_merged)?;
    Ok((target, undo_parts(plans.take())))
}

/// Changes the merged `a` after the merge as `change` says.
fn change_merged(change: &str, a: &Path) -> Result<(), Box<dyn Error>> {
    match change {
        "linked" => assert_eq!(fs::metadata(a)?.nlink(), 2, "the merge keeps the link"),
        "edited" => fs::write(a, b"edited after the merge")?,
        "touched" => set_times(a, 1_000_000_000, 0)?,
        "replaced" => {
            let merged = fs::metadata(a)?;
            let other = a.with_file_name("a.new");
            fs::write(&other, fs::read(a)?)?;
            set_times(
                &other,
                u64::try_from(merged.mtime())?,
                u32::try_from(merged.mtime_nsec())?,
            )?;
            fs::rename(&other, a)?;
        }
        _ => {}
    }
    Ok(())
}

fn check_left_unchanged(change: &str) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let _share =
        (change == "on a share").then(|| FsQuirks::install(root.path(), Quirks::cifs_posix()));
    let (target, parts) = merge_docs(root.path(), change)?;
    let [a, b] = ["docs/a.txt", "docs/b.txt"].map(|path| target.join(path));
    change_merged(change, &a)?;
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

    let events = run_undo(parts, staged_lookup(staged));

    assert_files(
        &target,
        &[("docs/b.txt", "old b"), ("docs/stays.txt", "keep me")],
        change,
    )?;
    if matches!(change, "untouched" | "on a share" | "linked") {
        assert_eq!(fs::read(&a)?, b"old a", "{change}");
        let mut undone = restored(&events).ok_or("not restored")?;
        undone.sort_by_key(Location::display_name);
        assert_eq!(undone, [Location::local(&a), Location::local(&b)]);
        assert!(kept_in_trash(&events).is_empty());
        return Ok(());
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
    assert_kept_in_trash(&events, (&trash, "a.txt"), &a, change);
    assert_eq!(fs::read(trash.join("files/a.txt"))?, b"old a", "{change}");
    Ok(())
}

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
        check_left_unchanged(change)?;
    }
    Ok(())
}

/// A file or `directory` replace, and what changes before or while its
/// undo runs.
#[derive(Clone, Copy)]
struct Replaced {
    directory: bool,
    change: &'static str,
}

impl Replaced {
    fn row(self) -> String {
        let kind = if self.directory { "folder" } else { "file" };
        format!("{} {kind}", self.change)
    }

    fn write(self, source: &Path, target: &Path) -> io::Result<()> {
        if self.directory {
            write_files(source, &[("a.txt", "new a"), ("sub/b.txt", "new b")])?;
            write_files(target, &[("old.txt", "old")])
        } else {
            fs::write(source, b"new")?;
            fs::write(target, b"old")
        }
    }

    /// Placed items are recorded with a forced stat on any mount, not only
    /// on a share this process just wrote to.
    fn recording_quirks(self) -> Option<Quirks> {
        match self.change {
            "untouched on a share" => Some(Quirks::cifs_posix()),
            "untouched under cached change times" => Some(Quirks {
                cached_change_times: Some(Duration::from_secs(1)),
                ..Quirks::default()
            }),
            _ => None,
        }
    }

    fn edit_after_the_replace(self, target: &Path) -> io::Result<()> {
        match (self.directory, self.change) {
            (false, "edited") => fs::write(target, b"edited"),
            (true, "edited") => fs::write(target.join("sub/b.txt"), b"edited b"),
            (true, "added to") => fs::write(target.join("sub/c.txt"), b"added"),
            _ => Ok(()),
        }
    }

    fn original(self, root: &Path, target: &Path) -> Result<RestoreEntry, Box<dyn Error>> {
        if self.directory {
            staged_trash_folder(root, "target", target, &[("old.txt", "old")])
        } else {
            staged_trash_fixture(root, "target", target, b"old")
        }
    }

    fn undo_quirks(self, target: &Path) -> Option<Quirks> {
        let edited = target.join("sub/b.txt");
        match self.change {
            "undeletable" => Some(Quirks {
                fail: vec![(Op::Unlink, "a.txt", 0, Errno::ACCESS)],
                ..Quirks::default()
            }),
            "still open in another program" => Some(Quirks {
                mount_type: Some(("nfs4", "rw,vers=4.2")),
                silly_rename: Some(super::super::super::fs_quirks::Silly::nfs()),
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
    }
}

/// Replaces `target` with `source` through Trash, returning what it placed.
fn replace_through_trash(
    source: &Path,
    target: &Path,
    row: &str,
) -> Result<PlacedSnapshot, Box<dyn Error>> {
    let replaced = RefCell::new(None);
    glib::MainContext::default().block_on(replace_local_with(
        gio::File::for_path(source),
        gio::File::for_path(target),
        false,
        gio::Cancellable::new(),
        None,
        super::super::super::super::Copying::default(),
        &|original, placed| *replaced.borrow_mut() = Some((original, placed)),
    ))?;
    let Some((ReplacedOriginal::Trashed(_), Some(placed))) = replaced.take() else {
        return Err(format!("{row}: the original was not replaced through Trash").into());
    };
    Ok(placed)
}

fn check_replace_undo(case: Replaced) -> Result<(), Box<dyn Error>> {
    let (row, change) = (case.row(), case.change);
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    case.write(&source, &target)?;
    let _cached = case
        .recording_quirks()
        .map(|quirks| FsQuirks::install(root.path(), quirks));
    let placed = replace_through_trash(&source, &target, &row)?;
    case.edit_after_the_replace(&target)?;
    let original = case.original(root.path(), &target)?;
    let trash = trash_in(root.path());
    let location = Location::local(&target);
    let _scope = case
        .undo_quirks(&target)
        .map(|quirks| FsQuirks::install(root.path(), quirks));
    let still_open = change == "still open in another program";
    let _clock = still_open.then(|| ClockGuard::set(std::time::SystemTime::now()));
    let _held = still_open.then(|| hold(&target.join("sub/b.txt")));

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

    let kept_in = (trash.as_path(), "target");
    match change {
        "still open in another program" => assert_still_open(&events, &trash, &row),
        "undeletable" => {
            assert_undeletable(&events, &target, &trash, &row)?;
            assert_eq!(restored(&undo()), Some(vec![location]));
            assert_eq!(names(&target)?, ["old.txt"]);
        }
        _ if change.starts_with("untouched") => {
            assert_eq!(restored(&events), Some(vec![location]), "{row}");
            assert_original_back(case, &target, &trash, &row)?;
        }
        _ => {
            assert_eq!(restored(&events), Some(Vec::new()), "{row}");
            assert_kept_in_trash(&events, kept_in, &target, &row);
            assert_changed_left(case, &target, &row)?;
        }
    }
    Ok(())
}

fn completed_with_errors<'a>(events: &'a [OperationEvent], row: &str) -> &'a str {
    let Some(OperationEvent::CompletedWithErrors { message, .. }) = events.last() else {
        panic!("{row}: {events:?}");
    };
    message
}

fn assert_still_open(events: &[OperationEvent], trash: &Path, row: &str) {
    let message = completed_with_errors(events, row);
    let error = rust_i18n::t!(
        "Could not delete “%{name}”: %{error}",
        name = "sub",
        error = rust_i18n::t!(
            "A file in it is still open in %{program}. Close it there and try again.",
            program = "Text Editor"
        )
    );
    assert!(message.contains(&*error), "{message}");
    assert!(kept_in_trash(events).is_empty());
    assert!(trash.join("info/target.trashinfo").exists());
}

fn assert_undeletable(
    events: &[OperationEvent],
    target: &Path,
    trash: &Path,
    row: &str,
) -> io::Result<()> {
    let message = completed_with_errors(events, row);
    let denied = crate::services::io_error_detail(&io::Error::from_raw_os_error(
        Errno::ACCESS.raw_os_error(),
    ));
    let error = rust_i18n::t!(
        "Could not delete “%{name}”: %{error}",
        name = "a.txt",
        error = denied
    );
    assert!(message.contains(&*error), "{message}");
    assert_eq!(names(target)?, ["a.txt"]);
    assert!(kept_in_trash(events).is_empty());
    assert!(trash.join("info/target.trashinfo").exists());
    Ok(())
}

fn assert_original_back(case: Replaced, target: &Path, trash: &Path, row: &str) -> io::Result<()> {
    let restored = if case.directory {
        assert_eq!(names(target)?, ["old.txt"], "{row}");
        fs::read(target.join("old.txt"))?
    } else {
        fs::read(target)?
    };
    assert_eq!(restored, b"old", "{row}");
    assert!(!trash.join("info/target.trashinfo").exists(), "{row}");
    Ok(())
}

fn assert_changed_left(case: Replaced, target: &Path, row: &str) -> io::Result<()> {
    match (case.directory, case.change) {
        (false, _) => assert_eq!(fs::read(target)?, b"edited"),
        (true, "edited during the undo") => {
            assert_eq!(fs::read(target.join("sub/b.txt"))?, b"edited b");
        }
        (true, change) => {
            assert_eq!(names(target)?, ["a.txt", "sub"], "{row}");
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
    Ok(())
}

#[test]
fn undoing_a_replace_deletes_what_it_placed_only_if_all_of_it_is_unchanged()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let file = |change| Replaced {
        directory: false,
        change,
    };
    let folder = |change| Replaced {
        directory: true,
        change,
    };
    let rows = [
        file("untouched"),
        file("edited"),
        file("untouched on a share"),
        file("untouched under cached change times"),
        folder("untouched"),
        folder("edited"),
        folder("added to"),
        folder("edited during the undo"),
        folder("undeletable"),
        folder("still open in another program"),
    ];
    for case in rows {
        check_replace_undo(case)?;
    }
    Ok(())
}

type MakeQuirks = Box<dyn Fn() -> Quirks>;

/// A merged item at the root's `merged.txt`, or in its `folder`, and the
/// staged original of where it was placed.
fn check_compared_as_moved(
    (mount, quirks, change, undone): (&str, MakeQuirks, &str, bool),
    epoch: SystemTime,
    seconds: u64,
) -> Result<(), Box<dyn Error>> {
    let row = format!("{mount}, {change}");
    // Waiting for a tick to pass moves it on.
    let _clock = ClockGuard::set(epoch + Duration::from_secs(seconds) + Duration::from_millis(500));
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
        "evicted" => super::super::super::fs_quirks::renumbered(&merged),
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
        let trash = trash_in(root.path());
        assert_kept_in_trash(&events, (&trash, name), &placed_at, &row);
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
    for row in rows {
        check_compared_as_moved(row, epoch, seconds)?;
    }
    Ok(())
}

fn check_cancelled_compare(
    after_a_kept_item: bool,
    epoch: SystemTime,
    seconds: u64,
) -> Result<(), Box<dyn Error>> {
    let _clock = ClockGuard::set(epoch + Duration::from_secs(seconds) + Duration::from_millis(500));
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

    let parts = (Vec::new(), overwritten, placed);
    let events = undo_merge(1616, parts, cancellable, staged_lookup(staged));

    let kept = if after_a_kept_item {
        vec![Location::local(&edited)]
    } else {
        Vec::new()
    };
    assert_eq!(
        cancel_outcome(&events, ""),
        [kept, Vec::new(), vec![Location::local(&merged)]]
    );
    assert_eq!(fs::read(&merged)?, b"merged");
    assert_eq!(
        fs::read(trash_in(root.path()).join("files/merged.txt"))?,
        b"original"
    );
    Ok(())
}

#[test]
fn a_merge_undo_cancelled_while_it_compares_an_item_leaves_it_whole() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let epoch = SystemTime::UNIX_EPOCH;
    let seconds = SystemTime::now().duration_since(epoch)?.as_secs() / 2 * 2;
    for after_a_kept_item in [false, true] {
        check_cancelled_compare(after_a_kept_item, epoch, seconds)?;
    }
    Ok(())
}
