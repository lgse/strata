// SPDX-License-Identifier: MIT

use super::*;

const CLOCK_START: Duration = Duration::from_secs(1_800_000_000);

type Holds = Arc<std::sync::Mutex<Vec<HoldGuard>>>;

fn releasing(holds: &Holds) -> Racer {
    let holds = holds.clone();
    Box::new(move || holds.lock().expect("the holds").clear())
}

fn folder_with_an_open_file(path: &Path, holds: &Holds) -> io::Result<()> {
    fs::create_dir(path)?;
    fs::write(path.join("open.txt"), b"open elsewhere")?;
    let held = hold(&path.join("open.txt"));
    holds
        .lock()
        .map_err(|_| io::Error::other("the holds"))?
        .push(held);
    Ok(())
}

fn delete_permanently(folders: &[PathBuf]) -> Vec<OperationEvent> {
    delete(folders, true)
}

fn delete(folders: &[PathBuf], permanent: bool) -> Vec<OperationEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    glib::MainContext::default().block_on(run_deletion_in(
        Rc::new(SystemTrash),
        OperationRequestId(1590),
        folders
            .iter()
            .map(|folder| DeletionTarget::from_entry(&directory_entry(folder)))
            .collect(),
        permanent,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
        gio::Cancellable::new(),
    ));
    events.take()
}

fn waited_since(start: SystemTime) -> Result<Duration, Box<dyn Error>> {
    Ok(super::super::super::copy_engine::clock_for_test().duration_since(start)?)
}

#[derive(Clone, Copy, Debug)]
enum Holding {
    Unnamed,
    Program,
    StrataRead,
    StrataOperation,
    TheDeleteItself,
}

#[derive(Clone, Copy, Debug)]
enum Meanwhile {
    StaysOpen(Holding),
    Closes,
    ClosesAsTheWaitEnds,
    IsSwappedForAnEmptyFolder,
}

const GONE_AFTER_THE_CLOSE: Duration = Duration::from_millis(100);

fn meanwhile_racer(
    meanwhile: Meanwhile,
    folder: &Path,
    holds: &Holds,
    wait_ends: SystemTime,
) -> Option<(Op, &'static str, usize, Racer)> {
    let (holds, folder) = (holds.clone(), folder.to_path_buf());
    match meanwhile {
        Meanwhile::StaysOpen(_) => None,
        Meanwhile::Closes => Some((Op::Unlink, "folder", 0, releasing(&holds))),
        Meanwhile::ClosesAsTheWaitEnds => Some((
            Op::Unlink,
            "folder",
            1,
            Box::new(move || {
                super::super::super::copy_engine::set_clock_for_test(Some(wait_ends));
                holds.lock().expect("the holds").clear();
                let gone = wait_ends + GONE_AFTER_THE_CLOSE;
                super::super::super::copy_engine::set_clock_for_test(Some(gone));
            }),
        )),
        Meanwhile::IsSwappedForAnEmptyFolder => Some((
            Op::Unlink,
            "folder",
            0,
            Box::new(move || {
                fs::rename(&folder, folder.with_extension("moved")).expect("move the folder");
                fs::create_dir(&folder).expect("make another folder");
            }),
        )),
    }
}

/// Whether `folder` holds nothing but one other program's leftover.
fn leftovers_only(folder: &Path) -> Result<bool, Box<dyn Error>> {
    let left = names(folder)?;
    Ok(left.len() <= 1
        && left
            .iter()
            .all(|name| foreign_leftover(OsStr::new(name)).is_some()))
}

/// What of Strata's own keeps the open file in use, as `holding` says.
struct InUse {
    _read: Option<file_activity::Hold>,
    _other_operation: Option<file_activity::Activity>,
    _this_delete: Option<file_activity::Activity>,
}

impl InUse {
    fn start(holding: Holding, real_folder: &Path) -> Result<Self, Box<dyn Error>> {
        use Holding::*;
        let read_by_strata = real_folder.join("open.txt");
        let _read = matches!(holding, StrataRead)
            .then(|| file_activity::hold([&read_by_strata], || {}))
            .transpose()?;
        let _other_operation = matches!(holding, StrataOperation).then(|| {
            file_activity::begin(file_activity::OperationScope {
                roots: vec![read_by_strata.clone()],
                folders: Vec::new(),
            })
        });
        let _this_delete = matches!(holding, TheDeleteItself).then(|| {
            file_activity::begin(file_activity::OperationScope {
                roots: vec![real_folder.to_path_buf(), read_by_strata.clone()],
                folders: Vec::new(),
            })
        });
        Ok(Self {
            _read,
            _other_operation,
            _this_delete,
        })
    }
}

/// Why a folder kept open by `holding` on `mount` could not be deleted.
fn still_open_reason(holding: Holding, mount: &str) -> String {
    use Holding::*;
    match (holding, mount) {
        (Program, _) => rust_i18n::t!(
            "A file in it is still open in %{program}. Close it there and try again.",
            program = "Text Editor"
        )
        .into_owned(),
        (StrataRead | StrataOperation, _) => crate::i18n::tr(
            "A file in it is still in use by Strata. Try again when its other operation or search ends.",
        ),
        (Unnamed | TheDeleteItself, "cifs") => {
            crate::i18n::tr("A file in it is still open in a program on this or another computer.")
        }
        (Unnamed | TheDeleteItself, "cifs-posix") => crate::services::io_error_detail(
            &io::Error::from_raw_os_error(rustix::io::Errno::ACCESS.raw_os_error()),
        ),
        (Unnamed | TheDeleteItself, _) => crate::i18n::tr(
            "A file in it is still open in another program. Close it and try again.",
        ),
    }
}

/// The delete waited out the mount's whole budget, kept the folder with
/// only the leftover, and named why.
fn assert_stays_open(
    events: &[OperationEvent],
    (holding, mount): (Holding, &str),
    scope: &FsQuirks,
    row: &str,
) {
    let line = rust_i18n::t!(
        "Could not delete “%{name}”: %{error}",
        name = "folder",
        error = still_open_reason(holding, mount)
    );
    assert!(
        matches!(
            events.last(),
            Some(OperationEvent::CompletedWithErrors { message, .. })
                if message.contains(&*line)
        ),
        "{row}: {events:?}"
    );
    let scanned = scope.count(Op::Holders);
    let expected = match holding {
        Holding::StrataRead | Holding::StrataOperation => 0,
        Holding::Unnamed | Holding::Program | Holding::TheDeleteItself => 1,
    };
    assert_eq!(scanned, expected, "{row}");
}

/// A mount's family, its preset, and how long it waits for open files.
type Waiting = (&'static str, Preset, Duration);

fn check_leftover_wait(
    (mount, preset, budget): Waiting,
    meanwhile: Meanwhile,
) -> Result<(), Box<dyn Error>> {
    use Meanwhile::*;
    let row = format!("{mount}, {meanwhile:?}");
    let start = SystemTime::UNIX_EPOCH + CLOCK_START;
    let _clock = ClockGuard::set(start);
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    let holds = Holds::default();
    let holding = match meanwhile {
        StaysOpen(holding) => holding,
        _ => Holding::Unnamed,
    };
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            after: meanwhile_racer(meanwhile, &folder, &holds, start + budget)
                .into_iter()
                .collect(),
            holder: matches!(holding, Holding::Program)
                .then(|| Holder::Program("Text Editor".to_owned())),
            ..preset()
        },
    );
    folder_with_an_open_file(&folder, &holds)?;
    let _in_use = InUse::start(holding, &folder.canonicalize()?)?;

    let events = delete_permanently(std::slice::from_ref(&folder));

    let waited = waited_since(start)?;
    let deleted = matches!(events.last(), Some(OperationEvent::Deleted { .. }));
    match meanwhile {
        Closes | ClosesAsTheWaitEnds => {
            assert!(deleted, "{row}: {events:?}");
            assert!(!folder.exists(), "{row}");
            if let Closes = meanwhile {
                assert!(waited < budget, "{row}: {waited:?}");
            }
        }
        StaysOpen(holding) => {
            assert!(!deleted, "{row}: {events:?}");
            assert_eq!(waited, budget, "{row}");
            assert!(leftovers_only(&folder)?, "{row}");
            assert_stays_open(&events, (holding, mount), &scope, &row);
        }
        IsSwappedForAnEmptyFolder => {
            assert!(!deleted, "{row}: {events:?}");
            assert!(names(&folder)?.is_empty(), "{row}");
            assert!(leftovers_only(&folder.with_extension("moved"))?, "{row}");
        }
    }
    Ok(())
}

#[test]
fn a_folder_only_leftovers_keep_goes_once_they_do_within_the_mount_s_wait()
-> Result<(), Box<dyn Error>> {
    use Holding::*;
    use Meanwhile::*;
    let _serial = serial()?;
    let smb = Duration::from_millis(1500);
    let fuse_and_nfs = Duration::from_secs(2);
    let renamed = |holding| {
        [
            StaysOpen(Unnamed),
            StaysOpen(holding),
            Closes,
            ClosesAsTheWaitEnds,
            IsSwappedForAnEmptyFolder,
        ]
    };
    let rows: [(Waiting, &[Meanwhile]); 6] = [
        (("nfs3", Quirks::nfs3, fuse_and_nfs), &renamed(StrataRead)),
        (("nfs42", Quirks::nfs42, fuse_and_nfs), &renamed(Program)),
        (("cifs", Quirks::cifs, smb), &renamed(StrataOperation)),
        (
            ("cifs-posix", Quirks::cifs_posix, smb),
            &[
                StaysOpen(Unnamed),
                StaysOpen(Program),
                Closes,
                IsSwappedForAnEmptyFolder,
            ],
        ),
        (
            ("sshfs", Quirks::sshfs, fuse_and_nfs),
            &renamed(TheDeleteItself),
        ),
        (
            ("exfat-fuse", Quirks::exfat_fuse, fuse_and_nfs),
            &renamed(Unnamed),
        ),
    ];
    for (waiting, cases) in rows {
        for &meanwhile in cases {
            check_leftover_wait(waiting, meanwhile)?;
        }
    }
    Ok(())
}

#[test]
fn one_wait_for_open_files_covers_every_folder_of_a_delete_or_trash() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let smb_folders_refused = || Quirks {
        folder_renames_refused_below: Some(Duration::from_secs(1)),
        ..Quirks::cifs()
    };
    let rows: [(&str, Preset, bool, Duration); 2] = [
        ("nfs3, Delete", Quirks::nfs3, true, Duration::from_secs(2)),
        (
            "cifs, Trash",
            smb_folders_refused,
            false,
            Duration::from_millis(1500),
        ),
    ];
    for (row, preset, permanent, budget) in rows {
        let start = SystemTime::UNIX_EPOCH + CLOCK_START;
        let _clock = ClockGuard::set(start);
        let root = tempfile::tempdir()?;
        let _scope = FsQuirks::install(root.path(), preset());
        let holds = Holds::default();
        let folders = [root.path().join("first"), root.path().join("second")];
        for folder in &folders {
            folder_with_an_open_file(folder, &holds)?;
        }

        let events = delete(&folders, permanent);

        assert!(
            matches!(
                events.last(),
                Some(OperationEvent::CompletedWithErrors { deleted_locations, .. })
                    if deleted_locations.is_empty()
            ),
            "{row}: {events:?}"
        );
        assert!(folders.iter().all(|folder| folder.is_dir()), "{row}");
        assert_eq!(waited_since(start)?, budget, "{row}");
    }
    Ok(())
}

#[test]
fn a_folder_kept_by_an_open_file_does_not_stop_the_rest_of_the_delete() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let _clock = ClockGuard::set(SystemTime::UNIX_EPOCH + CLOCK_START);
    let root = tempfile::tempdir()?;
    let _scope = FsQuirks::install(root.path(), Quirks::nfs3());
    let tree = root.path().join("tree");
    let holds = Holds::default();
    fs::create_dir(&tree)?;
    folder_with_an_open_file(&tree.join("open"), &holds)?;
    let rest = tree.join("rest");
    fs::create_dir(&rest)?;
    for index in 0..200 {
        fs::write(rest.join(format!("{index}.txt")), b"deletable")?;
    }

    let events = delete_permanently(std::slice::from_ref(&tree));

    let line = rust_i18n::t!(
        "Could not delete “%{name}”: %{error}",
        name = "open",
        error = crate::i18n::tr(
            "A file in it is still open in another program. Close it and try again."
        )
    );
    assert!(
        matches!(
            events.last(),
            Some(OperationEvent::CompletedWithErrors { message, .. }) if message.contains(&*line)
        ),
        "{events:?}"
    );
    assert!(
        !rest.exists(),
        "left behind: {:?}",
        names(&rest).unwrap_or_default().len()
    );
    assert_eq!(names(&tree)?, ["open"]);
    assert!(
        names(&tree.join("open"))?
            .iter()
            .all(|name| foreign_leftover(OsStr::new(name)).is_some())
    );
    Ok(())
}

#[test]
fn an_error_that_stops_a_delete_is_reported_over_an_earlier_open_folder()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let _clock = ClockGuard::set(SystemTime::UNIX_EPOCH + CLOCK_START);
    let root = tempfile::tempdir()?;
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            fail: vec![(Op::Unlink, "broken", 0, rustix::io::Errno::IO)],
            ..Quirks::nfs3()
        },
    );
    let holds = Holds::default();
    let open = root.path().join("open");
    folder_with_an_open_file(&open, &holds)?;
    let broken = root.path().join("broken");
    fs::write(&broken, b"keep")?;

    // One worker takes the newest job first, so the open folder is refused before the failure.
    let error = parallel_delete_local_blocking_with_workers(
        vec![delete_root(&broken)?, delete_root(&open)?],
        Arc::new(AtomicBool::new(false)),
        1,
        &Deadline::default(),
    )
    .expect_err("the failed unlink");

    assert!(error.contains("broken"), "{error}");
    assert!(!error.contains("open"), "{error}");
    assert!(broken.exists() && open.is_dir());
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Hidden {
    Leftover { closed_after: Option<Op> },
    LeftoverBesideAnEarlierOne,
    NamedLikeOne,
}

const EARLIER: &str = ".__smbfile_silly1";

/// Leaves a hidden name in `folder` as `hidden` says: another program's
/// leftover of an open file unlinked there, or an ordinary file named alike.
fn plant_hidden(hidden: Hidden, folder: &Path, holds: &Holds) -> Result<(), Box<dyn Error>> {
    if let Hidden::NamedLikeOne = hidden {
        fs::create_dir(folder)?;
        fs::write(folder.join(".nfs000000000000005e000000de"), b"ordinary")?;
        return Ok(());
    }
    folder_with_an_open_file(folder, holds)?;
    if let Hidden::LeftoverBesideAnEarlierOne = hidden {
        fs::write(folder.join(EARLIER), b"earlier")?;
    }
    let dir = open_local_parent_directory(folder)?;
    super::super::super::fs_ops::fs().unlink(
        super::super::super::fs_ops::Entry::new(&dir, OsStr::new("open.txt")),
        rustix::fs::AtFlags::empty(),
    )?;
    Ok(())
}

fn assert_planted(hidden: Hidden, folder: &Path, row: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let hidden_names = names(folder)?;
    if let Hidden::LeftoverBesideAnEarlierOne = hidden {
        assert_eq!(hidden_names.len(), 2, "{row}: {hidden_names:?}");
        assert_eq!(fs::read(folder.join(EARLIER))?, b"earlier", "{row}");
    } else {
        assert_eq!(hidden_names.len(), 1, "{row}: {hidden_names:?}");
    }
    Ok(hidden_names)
}

fn unlinked_names(scope: &FsQuirks) -> Vec<OsString> {
    scope
        .calls()
        .into_iter()
        .filter(|call| call.op == Op::Unlink)
        .flat_map(|call| call.names)
        .collect()
}

/// The delete unlinked no leftover, finishing once the file closed or
/// leaving the leftovers alone in the folder.
fn assert_leftovers_spared(
    events: &[OperationEvent],
    (folder, hidden_names): (&Path, &[String]),
    unlinked: &[OsString],
    (closed, row): (bool, &str),
) -> Result<(), Box<dyn Error>> {
    assert!(
        hidden_names
            .iter()
            .all(|name| !unlinked.contains(&name.into())),
        "{row}: {unlinked:?}"
    );
    if closed {
        assert!(
            matches!(events.last(), Some(OperationEvent::Deleted { .. })),
            "{row}: {events:?}"
        );
        assert!(!folder.exists(), "{row}");
        return Ok(());
    }
    assert!(
        matches!(
            events.last(),
            Some(OperationEvent::CompletedWithErrors { .. })
        ),
        "{row}: {events:?}"
    );
    assert_eq!(names(folder)?, hidden_names, "{row}");
    Ok(())
}

fn check_hidden_leftover(
    mount: &str,
    preset: Preset,
    hidden: Hidden,
) -> Result<(), Box<dyn Error>> {
    let row = format!("{mount}, {hidden:?}");
    let start = SystemTime::UNIX_EPOCH + CLOCK_START;
    let _clock = ClockGuard::set(start);
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    let holds = Holds::default();
    let closed_after = match hidden {
        Hidden::Leftover { closed_after } => closed_after,
        Hidden::LeftoverBesideAnEarlierOne | Hidden::NamedLikeOne => None,
    };
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            after: closed_after
                .map(|op| (op, "folder", 0, releasing(&holds)))
                .into_iter()
                .collect(),
            ..preset()
        },
    );
    plant_hidden(hidden, &folder, &holds)?;
    let hidden_names = assert_planted(hidden, &folder, &row)?;
    fs::write(folder.join("ordinary.txt"), b"ordinary")?;
    scope.clear_calls();

    let events = delete_permanently(std::slice::from_ref(&folder));

    let unlinked = unlinked_names(&scope);
    assert!(
        unlinked.contains(&"ordinary.txt".into()),
        "{row}: {unlinked:?}"
    );
    if let Hidden::NamedLikeOne = hidden {
        assert!(!folder.exists(), "{row}: {events:?}");
        return Ok(());
    }
    assert_leftovers_spared(
        &events,
        (&folder, &hidden_names),
        &unlinked,
        (closed_after.is_some(), &row),
    )
}

#[test]
fn a_recursive_delete_never_unlinks_another_program_s_leftover() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let leftover = |closed_after| Hidden::Leftover { closed_after };
    let rows: [(&str, Preset, Hidden); 10] = [
        ("nfs3", Quirks::nfs3, leftover(None)),
        ("nfs3", Quirks::nfs3, leftover(Some(Op::Unlink))),
        ("nfs3", Quirks::nfs3, leftover(Some(Op::List))),
        ("cifs", Quirks::cifs, leftover(None)),
        ("cifs", Quirks::cifs, leftover(Some(Op::Unlink))),
        ("cifs", Quirks::cifs, Hidden::LeftoverBesideAnEarlierOne),
        ("sshfs", Quirks::sshfs, leftover(None)),
        ("sshfs", Quirks::sshfs, leftover(Some(Op::Unlink))),
        ("ext4", Quirks::ext4, Hidden::NamedLikeOne),
        ("exfat-fuse", Quirks::exfat_fuse, Hidden::NamedLikeOne),
    ];
    for (mount, preset, hidden) in rows {
        check_hidden_leftover(mount, preset, hidden)?;
    }
    Ok(())
}
