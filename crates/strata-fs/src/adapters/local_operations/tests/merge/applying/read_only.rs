// SPDX-License-Identifier: MIT

use std::os::unix::fs::PermissionsExt;

use super::*;

fn mode(path: &Path) -> Option<u32> {
    fs::metadata(path)
        .ok()
        .map(|metadata| metadata.mode() & 0o7777)
}

fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// Opens each of `folders` that is still a folder again.
fn reopen(folders: &[&PathBuf]) -> io::Result<()> {
    for folder in folders {
        if folder.is_dir() {
            set_mode(folder, 0o755)?;
        }
    }
    Ok(())
}

#[test]
fn merge_places_a_read_only_incoming_folder() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(source.join("incoming"))?;
    fs::create_dir_all(&target)?;
    fs::write(source.join("incoming/file.txt"), b"new")?;
    fs::write(target.join("stays.txt"), b"keep me")?;
    let incoming = source.join("incoming");
    set_mode(&incoming, 0o555)?;

    let result = merge_into(&source, &target, &no_plan);

    let placed = fs::metadata(target.join("incoming")).map(|metadata| metadata.mode() & 0o7777);
    reopen(&[&incoming, &target.join("incoming")])?;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(placed?, 0o555);
    assert_files(
        &target,
        &[("incoming/file.txt", "new"), ("stays.txt", "keep me")],
        "",
    )?;
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Undo {
    Whole,
    Refilled,
    Cancelled,
    IntoLockedTarget,
}

/// The modes of the target and its merged `incoming` and `incoming/inner`,
/// and what each of those two still holds.
type Left = ([Option<u32>; 3], [Option<Vec<String>>; 2]);

impl Undo {
    /// Before the undo unlinks `file.txt`, refills `inner` or cancels.
    fn quirks(self, inner: &Path, cancellable: &gio::Cancellable) -> Quirks {
        let (refill, cancel) = (inner.join("theirs.txt"), cancellable.clone());
        let racer: Racer = Box::new(move || match self {
            Undo::Refilled => drop(fs::write(&refill, b"theirs")),
            _ => cancel.cancel(),
        });
        let mut quirks = without_trash(Quirks::default());
        if let Undo::Refilled | Undo::Cancelled = self {
            quirks.before = vec![(Op::Unlink, "file.txt", 0, racer)];
        }
        quirks
    }

    fn left(self) -> Left {
        let inner = || Some(vec!["inner".to_owned()]);
        match self {
            Undo::Whole => ([Some(0o755), None, None], [None, None]),
            Undo::Refilled => (
                [Some(0o755), Some(0o555), Some(0o555)],
                [inner(), Some(vec!["theirs.txt".to_owned()])],
            ),
            Undo::Cancelled => (
                [Some(0o755), Some(0o555), Some(0o555)],
                [inner(), Some(Vec::new())],
            ),
            Undo::IntoLockedTarget => ([Some(0o555), Some(0o555), None], [Some(Vec::new()), None]),
        }
    }
}

/// Merges a read-only `incoming/inner` folder pair into `target` without
/// Trash, returning what undo needs.
fn merge_read_only_folders(
    root: &Path,
    source: &Path,
    target: &Path,
) -> Result<(Result<(), glib::Error>, UndoParts), Box<dyn Error>> {
    fs::create_dir_all(source.join("incoming/inner"))?;
    fs::create_dir_all(target)?;
    fs::write(source.join("incoming/inner/file.txt"), b"new")?;
    fs::write(target.join("stays.txt"), b"keep me")?;
    for folder in ["incoming/inner", "incoming"] {
        set_mode(&source.join(folder), 0o555)?;
    }
    let _merging = FsQuirks::install(root, without_trash(Quirks::default()));
    let (plans, on_merged) = recorded_plans();
    let merged = merge_into(source, target, &on_merged);
    Ok((merged, undo_parts(plans.take())))
}

fn check_read_only_undo(undo: Undo) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    let (merged, parts) = merge_read_only_folders(root.path(), &source, &target)?;
    let (incoming, inner) = (target.join("incoming"), target.join("incoming/inner"));
    let cancellable = gio::Cancellable::new();
    let _undoing = FsQuirks::install(root.path(), undo.quirks(&inner, &cancellable));
    if let Undo::IntoLockedTarget = undo {
        set_mode(&target, 0o555)?;
    }
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();

    glib::MainContext::default().block_on(run_merge_undo(
        OperationRequestId(93),
        parts,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
        cancellable,
        staged_lookup(HashMap::new()),
    ));

    let modes = [&target, &incoming, &inner].map(|folder| mode(folder));
    let left = [&incoming, &inner].map(|folder| names(folder).ok());
    for folder in [&source.join("incoming"), &source.join("incoming/inner")] {
        set_mode(folder, 0o755)?;
    }
    reopen(&[&target, &incoming, &inner])?;
    let row = format!("{undo:?}: {:?}", events.borrow());
    assert!(merged.is_ok(), "{row}: {merged:?}");
    assert_eq!((modes, left), undo.left(), "{row}");
    if let Undo::Whole = undo {
        assert!(restored(&events.borrow()).is_some(), "{row}");
    }
    assert!(names(&target)?.contains(&"stays.txt".to_owned()), "{row}");
    Ok(())
}

#[test]
fn undoing_a_merge_without_trash_opens_only_its_own_read_only_folders_to_remove_them()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for undo in [
        Undo::Whole,
        Undo::Refilled,
        Undo::Cancelled,
        Undo::IntoLockedTarget,
    ] {
        check_read_only_undo(undo)?;
    }
    Ok(())
}
