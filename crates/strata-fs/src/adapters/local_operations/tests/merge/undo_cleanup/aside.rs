// SPDX-License-Identifier: MIT

use super::super::super::fs_quirks::{Aside, Silly};
use super::*;

fn reported_kept(events: &[OperationEvent]) -> Vec<KeptItem> {
    events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::ItemsReported { report, .. } => Some(report.kept.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn merge_request(id: u64, destination: &Path, source: &Path, move_sources: bool) -> PasteRequest {
    PasteRequest {
        id: OperationRequestId(id),
        destination: Location::local(destination),
        items: vec![PasteItem {
            source: Location::local(source),
            conflict: TransferConflict::Merge,
            target_name: None,
        }],
        move_sources,
    }
}

fn check_set_aside(trash: Trash, quirks: Quirks, move_source: bool) -> Result<(), Box<dyn Error>> {
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
    assert_files(
        &target,
        &[
            ("shared.txt", "new"),
            ("nested/deep/shared.txt", "new deep"),
            ("nested/stays.txt", "keep me"),
        ],
        "",
    )?;
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
    Ok(())
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
        check_set_aside(trash, quirks, move_source)?;
    }
    Ok(())
}

fn rewrite(path: PathBuf, contents: &'static [u8]) -> Racer {
    Box::new(move || fs::write(&path, contents).expect("rewrite the source"))
}

/// Sets up `source/x`, which the merge leaves out as a socket, or which is
/// rewritten twice as it is read when `moving`; the socket comes back.
fn left_nothing_source(
    source: &Path,
    moving: bool,
) -> io::Result<(Quirks, Option<std::os::unix::net::UnixListener>)> {
    let mut quirks = Quirks::default();
    if !moving {
        let socket = std::os::unix::net::UnixListener::bind(source.join("x"))?;
        return Ok((quirks, Some(socket)));
    }
    fs::write(source.join("x"), b"incoming")?;
    quirks.before = vec![
        (Op::Read, "x", 0, rewrite(source.join("x"), b"second")),
        (Op::Read, "x", 1, rewrite(source.join("x"), b"third")),
    ];
    Ok((quirks, None))
}

fn left_nothing_target(row: &str, target: &Path) -> Quirks {
    let mut quirks = without_trash(Quirks::default());
    if row == "put back fails" {
        quirks.fail = vec![(Op::Rename, "x", 1, Errno::IO)];
    }
    if row == "name taken meanwhile" {
        quirks.before = vec![(Op::Rename, "x", 1, rewrite(target.join("x"), b"theirs"))];
    }
    quirks
}

fn check_left_nothing(row: &str) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source)?;
    write_files(&target, &[("x", "original"), ("y", "kept")])?;
    let moving = row == "kept at its source";
    let (source_quirks, _socket) = left_nothing_source(&source, moving)?;
    let target_quirks = left_nothing_target(row, &target);
    let _source = FsQuirks::install(&source, source_quirks);
    let _target = FsQuirks::install(&target, target_quirks);

    let (result, _) = merge_through_trash(&source, &target, moving, real_copy());

    if matches!(row, "put back fails" | "name taken meanwhile") {
        let aside = aside_folder(&target)?.ok_or("the original stays aside")?;
        assert!(result.is_err(), "{row}");
        assert_files(&target, &[(&format!("{aside}/x"), "original")], row)?;
        if row == "name taken meanwhile" {
            assert_eq!(fs::read(target.join("x"))?, b"theirs", "{row}");
        }
        return Ok(());
    }
    assert!(result.is_ok(), "{row}: {result:?}");
    assert_eq!(names(&target)?, ["x", "y"], "{row}");
    assert_eq!(fs::read(target.join("x"))?, b"original", "{row}");
    if moving {
        assert_eq!(fs::read(source.join("x"))?, b"third", "{row}");
    }
    Ok(())
}

#[test]
fn without_trash_an_original_the_merge_left_nothing_in_place_of_stays() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    for row in [
        "left out",
        "kept at its source",
        "put back fails",
        "name taken meanwhile",
    ] {
        check_left_nothing(row)?;
    }
    Ok(())
}

/// A merge without Trash over an original another program holds open:
/// whether the copy fails, the source is left out as a socket, or the
/// original is replaced.
#[derive(Clone, Copy)]
struct StillOpen {
    row: &'static str,
    move_sources: bool,
    original: &'static str,
    preset: Preset,
    holder: Option<&'static str>,
}

impl StillOpen {
    fn describe(&self) -> String {
        format!(
            "{} {}, moving: {}, {:?}",
            self.row,
            self.original,
            self.move_sources,
            (self.preset)().mount_type
        )
    }

    fn replaced(&self) -> bool {
        self.row.starts_with("replaced")
    }

    fn copy_fails(&self) -> bool {
        self.row.starts_with("copy fails")
    }

    fn left_out(&self) -> bool {
        self.row.starts_with("left out")
    }

    /// Whether the mount lists the program's silly-renamed file.
    fn listed(&self) -> bool {
        !matches!(
            (self.preset)().silly_rename,
            Some(Silly {
                aside: Aside::Unlisted(_),
                ..
            })
        )
    }

    /// Whether the original goes back by a link that keeps its number.
    fn linked_back(&self, root: &Path) -> io::Result<bool> {
        let preset = (self.preset)();
        let links = matches!(preset.links, Support::Supported) && !preset.link_inodes;
        Ok(links && base_links_keep_their_number(root)?)
    }

    fn write(&self, source: &Path, target: &Path) -> io::Result<()> {
        write_files(source, &[("y.txt", "new y")])?;
        write_files(target, &[(self.original, "original")])?;
        if self.row == "replaced" {
            write_files(source, &[("also/z", "incoming")])?;
            write_files(target, &[("also/z", "original")])?;
        }
        let parent = source.join(self.original);
        fs::create_dir_all(
            parent
                .parent()
                .ok_or_else(|| io::Error::other("no parent"))?,
        )
    }

    fn expected_kept(&self, folder: Option<&str>, source: &Path, target: &Path) -> Vec<KeptItem> {
        let holder = self.holder();
        let cause = match holder {
            None if !self.listed() => KeptCause::Undeletable(Errno::ACCESS.raw_os_error()),
            holder => KeptCause::StillOpen(holder),
        };
        let mut expected: Vec<_> = folder
            .map(|folder| KeptItem {
                location: Location::local(target.join(folder)),
                cause,
            })
            .into_iter()
            .collect();
        if self.left_out() && self.move_sources {
            expected.push(KeptItem {
                location: Location::local(source.join(self.original)),
                cause: KeptCause::Skipped(crate::services::SpecialKind::Socket),
            });
        }
        expected
    }

    fn holder(&self) -> Option<crate::services::Holder> {
        self.holder
            .map(|program| crate::services::Holder::Program(program.to_owned()))
    }
}

/// The incoming `original` as a socket the merge leaves out, or as a file;
/// `None` where the base refuses Unix sockets.
fn incoming_original(
    case: &StillOpen,
    source: &Path,
) -> Result<Option<Option<std::os::unix::net::UnixListener>>, Box<dyn Error>> {
    if !case.left_out() {
        fs::write(source.join(case.original), b"incoming")?;
        return Ok(Some(None));
    }
    match std::os::unix::net::UnixListener::bind(source.join(case.original)) {
        Ok(socket) => Ok(Some(Some(socket))),
        Err(error) if matches!(Errno::from_io_error(&error), Some(Errno::PERM | Errno::IO)) => {
            eprintln!(
                "skipped on a base that refuses Unix sockets: {} {}, moving: {}",
                case.row, case.original, case.move_sources
            );
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn check_still_open(case: StillOpen) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let put_back_by_rename = !case.replaced() && !case.linked_back(root.path())?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    let (source, target) = (sources.join("run"), destination.join("run"));
    case.write(&source, &target)?;
    let Some(_socket) = incoming_original(&case, &source)? else {
        return Ok(());
    };
    let mut source_quirks = Quirks::default();
    if case.copy_fails() {
        source_quirks.fail = vec![(Op::OpenSource, "x", 0, Errno::IO)];
    }
    let _sources = FsQuirks::install(&sources, source_quirks);
    let _destination = FsQuirks::install(
        &destination,
        without_trash(Quirks {
            holder: case.holder(),
            ..(case.preset)()
        }),
    );
    let _clock = ClockGuard::set(std::time::SystemTime::now());
    let _held = hold(&target.join(case.original));
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        merge_request(1729, &destination, &source, case.move_sources),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    drive_until_transfer_settles(&events);

    let row = case.describe();
    let events = events.take();
    let folder = aside_folder(&target)?;
    assert_eq!(folder.is_none(), put_back_by_rename, "{row}: {events:?}");
    let expected = case.expected_kept(folder.as_deref(), &source, &target);
    assert_eq!(reported_kept(&events), expected, "{row}: {events:?}");
    let now_there = if case.replaced() {
        "incoming"
    } else {
        "original"
    };
    assert_files(&target, &[(case.original, now_there)], &row)?;
    if let Some(folder) = &folder {
        assert_nfs_leftovers(
            &target.join(folder).join(case.original),
            case.listed(),
            &row,
        )?;
    }
    assert_ending(
        &case,
        &events,
        (&source, &target),
        (folder.as_deref(), &row),
    )?;
    Ok(())
}

fn assert_ending(
    case: &StillOpen,
    events: &[OperationEvent],
    (source, target): (&Path, &Path),
    (folder, row): (Option<&str>, &str),
) -> io::Result<()> {
    match terminal_transfer(events) {
        Some(OperationEvent::TransferFailed { message, .. }) if case.copy_fails() => {
            assert!(
                folder.iter().all(|folder| !message.contains(folder)),
                "{row}: {message}"
            );
            assert_eq!(fs::read(source.join(case.original))?, b"incoming", "{row}");
        }
        Some(OperationEvent::Pasted { .. }) if !case.copy_fails() => {
            assert_moved_as_pasted(case, events, (source, target), row)?;
        }
        other => panic!("{row}: {other:?}"),
    }
    Ok(())
}

/// The folder set aside holds only the program's silly-renamed file, if
/// the mount lists it.
fn assert_nfs_leftovers(aside: &Path, listed: bool, row: &str) -> Result<(), Box<dyn Error>> {
    let leftovers = names(aside.parent().ok_or("no parent")?)?;
    let only_silly = leftovers.iter().all(|name| name.starts_with(".nfs"));
    assert!(
        leftovers.len() == usize::from(listed) && only_silly,
        "{row}: {leftovers:?}"
    );
    Ok(())
}

fn assert_moved_as_pasted(
    case: &StillOpen,
    events: &[OperationEvent],
    (source, target): (&Path, &Path),
    row: &str,
) -> io::Result<()> {
    assert_eq!(fs::read(target.join("y.txt"))?, b"new y", "{row}");
    for name in ["y.txt", case.original] {
        let gone = case.move_sources && (name == "y.txt" || case.replaced());
        assert_eq!(source.join(name).exists(), !gone, "{row}: {name}");
    }
    assert_eq!(
        item_moves(events).len(),
        usize::from(case.move_sources),
        "{row}"
    );
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
    let open = |row, move_sources, original, preset, holder| StillOpen {
        row,
        move_sources,
        original,
        preset,
        holder,
    };
    for case in [
        open("copy fails", false, "x", Quirks::nfs3 as Preset, editor),
        open("copy fails", false, "sub/x", Quirks::nfs3, editor),
        open("copy fails, no links", false, "x", without_links, editor),
        open(
            "copy fails, links numbered apart",
            false,
            "x",
            renumbered_links,
            editor,
        ),
        open("left out", false, "x", Quirks::nfs3, editor),
        open("left out", true, "sub/x", Quirks::nfs3, editor),
        open("replaced", true, "x", Quirks::nfs3, editor),
        open("replaced", true, "x", Quirks::cifs_posix, editor),
        open("replaced", true, "x", Quirks::cifs_posix, None),
        open("replaced", true, "sub/x", Quirks::nfs3, None),
        open("replaced", true, "sub/x", Quirks::cifs_posix, None),
    ] {
        check_still_open(case)?;
    }
    Ok(())
}

/// Writes a partial `a.txt` into the target, then fails with `failure`.
fn partial_copy(failure: gio::IOErrorEnum) -> CopyStep {
    Rc::new(move |_, target, _, _| {
        Box::pin(async move {
            let path = target
                .path()
                .ok_or_else(|| io_error("missing target path"))?;
            fs::write(path.join("a.txt"), b"partial").map_err(io_error)?;
            Err(glib::Error::new(failure, "injected copy failure"))
        })
    })
}

fn check_failed_without_trash(
    failure: gio::IOErrorEnum,
    reason: &str,
) -> Result<(), Box<dyn Error>> {
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

    let (result, plans) = merge_through_trash(&source, &target, false, partial_copy(failure));

    let error = result.expect_err("the copy failed");
    let folder = aside_folder(&target)?.ok_or("the displaced original was not kept")?;
    assert!(error.message().contains(&folder), "{error}");
    assert!(error.message().contains(reason), "{error}");
    assert_eq!(names(&target.join(&folder))?, ["a.txt"]);
    assert_files(
        &target,
        &[
            (&format!("{folder}/a.txt"), "old a"),
            ("a.txt", "partial"),
            ("b.txt", "old b"),
            ("stays.txt", "keep me"),
        ],
        "",
    )?;
    assert!(plans.iter().all(|plan| plan.overwritten.is_empty()));
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
        check_failed_without_trash(failure, reason)?;
    }
    Ok(())
}

const ASIDE_TARGET: [(&str, &str); 3] = [
    ("a.txt", "old a"),
    ("b.txt", "old b"),
    ("mount/c.txt", "old c"),
];

fn aside_quirks(case: &str) -> Quirks {
    match case {
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
    }
}

fn check_failed_aside(case: &str) -> Result<(), Box<dyn Error>> {
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
    write_files(&target, &ASIDE_TARGET)?;
    let quirks = FsQuirks::install(root.path(), without_trash(aside_quirks(case)));
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
    let copy: CopyStep = Rc::new(move |_, _, _, _| {
        copy_observed.set(true);
        Box::pin(async { Ok(MergeCopied::default()) })
    });

    let (result, plans) = merge_through_trash(&source, &target, false, copy);

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
    assert_files(&target, &ASIDE_TARGET, case)?;
    assert_eq!(names(&target)?, ["a.txt", "b.txt", "mount"], "{case}");
    assert!(plans.iter().all(|plan| plan.overwritten.is_empty()));
    Ok(())
}

#[test]
fn a_failed_aside_puts_every_original_back_before_merging() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for case in [
        "no safe placement",
        "second aside fails",
        "nested mount",
        "strict open crosses mount",
    ] {
        check_failed_aside(case)?;
    }
    Ok(())
}

/// Before the flush, removes what the merge published at `gone`, putting
/// another item there when `replaced`.
fn remove_published(gone: PathBuf, replaced: bool) -> Racer {
    Box::new(move || {
        if !gone.is_dir() {
            fs::remove_file(&gone).expect("remove copied file");
            if replaced {
                fs::write(&gone, b"replacement").expect("replace file");
            }
            return;
        }
        fs::remove_dir_all(&gone).expect("remove copied directory");
        if replaced {
            fs::create_dir(&gone).expect("replace directory");
        }
    })
}

fn check_published_gone(missing: &str, replaced: bool) -> Result<(), Box<dyn Error>> {
    let row = format!("{missing}, replaced {replaced}");
    let root = tempfile::tempdir()?;
    let source = root.path().join("sources/target");
    let destination = root.path().join("destination");
    let target = destination.join("target");
    write_files(&source, &[("nested/a.txt", "incoming")])?;
    fs::create_dir_all(target.join("nested"))?;
    let mut before = Vec::new();
    if missing.ends_with("(1).txt") {
        let taken = target.join("nested/a.txt");
        let racer: Racer = Box::new(move || fs::write(&taken, b"arrival").expect("take name"));
        before.push((Op::Rename, "a.txt", 0, racer));
    }
    before.push((
        Op::SyncFs,
        "*",
        0,
        remove_published(target.join(missing), replaced),
    ));
    let _quirks = FsQuirks::install(
        root.path(),
        without_trash(Quirks {
            before,
            ..Quirks::default()
        }),
    );
    let events = run_paste(merge_request(1619, &destination, &source, true));
    assert!(
        matches!(events.last(), Some(OperationEvent::Pasted { .. })),
        "{row}: {events:?}"
    );
    assert_eq!(
        reported_kept(&events),
        [KeptItem {
            location: Location::local(&source),
            cause: KeptCause::CopyGone
        }],
        "{row}"
    );
    assert_eq!(fs::read(source.join("nested/a.txt"))?, b"incoming", "{row}");
    Ok(())
}

#[test]
fn a_flushed_merge_move_keeps_sources_when_published_entries_disappear()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for missing in ["", "nested", "nested/a.txt", "nested/a (1).txt"] {
        for replaced in [false, true] {
            check_published_gone(missing, replaced)?;
        }
    }
    Ok(())
}

type Seen = Arc<std::sync::Mutex<Vec<bool>>>;

/// Records, at each flush, whether the originals are still aside in `target`.
fn aside_at_flush(target: PathBuf) -> (Seen, Racer) {
    let seen = Seen::default();
    let record = seen.clone();
    let racer: Racer = Box::new(move || {
        let kept = aside_folder(&target).is_ok_and(|folder| folder.is_some());
        record
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(kept);
    });
    (seen, racer)
}

fn check_flushed_before_deleting(flushed: bool) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    write_files(&source, &[("shared.txt", "new")])?;
    write_files(&target, &[("shared.txt", "old")])?;
    let (aside_during_flush, racer) = aside_at_flush(target.clone());
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
        return Ok(());
    }
    let aside = aside_folder(&target)?.ok_or("the originals stay aside")?;
    assert_files(
        root.path(),
        &[
            (&format!("target/{aside}/shared.txt"), "old"),
            ("source/shared.txt", "new"),
        ],
        "",
    )?;
    let error = result.expect_err("the originals and the source are kept");
    let message = error.message();
    assert!(
        message.contains(&aside) && message.contains("writes may still be pending"),
        "{error}"
    );
    Ok(())
}

#[test]
fn merge_originals_are_deleted_only_once_the_merge_is_flushed() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for flushed in [true, false] {
        check_flushed_before_deleting(flushed)?;
    }
    Ok(())
}
