// SPDX-License-Identifier: MIT

use std::os::unix::fs::MetadataExt;

use rustix::io::Errno;

use super::super::fs_ops::{Entry, fs};
use super::super::placement::{PlaceError, RenameFailure, rename_in_place};
use super::fs_quirks::{Op, Support, Trash};
use super::*;
use crate::{
    adapters::mount_lanes,
    model::hidden_names::{Leftover, classify},
    services::{
        Holder, KeptCause, KeptItem, LeftoverInside, RecoverRequest, RenameCause, RenamedItem,
        ReviewedLeftover, TransferReport,
    },
};

fn originals_in(folder: &Path) -> Result<(String, Vec<String>), Box<dyn Error>> {
    let found: Vec<_> = names(folder)?
        .into_iter()
        .filter(|name| classify(OsStr::new(name)) == Some(Leftover::Originals))
        .collect();
    let [originals] = found.as_slice() else {
        return Err(format!("one originals folder in {:?}", names(folder)?).into());
    };
    let inside = names(&folder.join(originals))?;
    Ok((originals.clone(), inside))
}

#[test]
fn a_copy_cut_off_by_an_unplug_leaves_only_its_stage() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.bin");
    fs::write(&source, b"copied")?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let target = destination.join("copy.bin");
    let _source = FsQuirks::install(root.path(), Quirks::ext4());
    let scope = FsQuirks::install(
        &destination,
        Quirks {
            tmpfile: Support::Rejected(Errno::OPNOTSUPP),
            short_io: Some(2),
            unplugged_after: Some((Op::Write, ".strata-copy-*", 1)),
            ..Quirks::ext4()
        },
    );
    let copy = || {
        glib::MainContext::default().block_on(copy_new_recursively(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            gio::Cancellable::new(),
        ))
    };
    let stat = |dir: &Path, name: &str| -> Result<Result<(), Errno>, Box<dyn Error>> {
        let dir = open_local_parent_directory(dir)?;
        Ok(fs()
            .stat(Entry::new(&dir, OsStr::new(name)), false)
            .map(drop))
    };

    assert!(copy().is_err());
    let left = names(&destination)?;
    let [stage] = left.as_slice() else {
        panic!("only the stage stays: {left:?}");
    };
    assert_eq!(classify(OsStr::new(stage)), Some(Leftover::IncompleteCopy));
    assert_eq!(fs::read(destination.join(stage))?, b"copi");
    let calls = scope.calls();
    let unplugged = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| call.op == Op::Write)
        .nth(1)
        .map(|(index, _)| index)
        .ok_or("the stage's second write")?;
    assert!(
        calls[unplugged + 1..]
            .iter()
            .any(|call| call.names.iter().any(|name| name == stage.as_str())),
        "{calls:?}"
    );
    assert_eq!(stat(&destination, stage)?, Err(Errno::IO));
    assert_eq!(stat(root.path(), "source.bin")?, Ok(()));
    assert!(
        fs().trash(&destination.join(stage), &gio::Cancellable::new())
            .is_err()
    );

    scope.clear_calls();
    assert!(copy().is_err());
    assert!(!scope.calls().is_empty());
    assert_eq!(names(&destination)?, left);

    scope.replug();
    assert_eq!(stat(&destination, stage)?, Ok(()));
    copy()?;
    assert_eq!(fs::read(&target)?, b"copied");
    assert_eq!(names(&destination)?, [stage.as_str(), "copy.bin"]);
    Ok(())
}

#[test]
fn an_unplug_after_setting_aside_keeps_the_original_under_its_name() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (quirks, moving) in [(Quirks::nfs3(), false), (Quirks::ext4(), true)] {
        let row = format!("moving: {moving}");
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::write(&source, b"new")?;
        fs::write(&target, b"old")?;
        let scope = FsQuirks::install(
            root.path(),
            Quirks {
                trash: Trash::Unsupported,
                unplugged_after: Some((Op::Rename, "target", 0)),
                ..quirks
            },
        );

        let result = glib::MainContext::default().block_on(replace_local(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            moving,
            gio::Cancellable::new(),
            None,
        ));

        assert!(result.is_err(), "{row}");
        scope.replug();
        let (originals, inside) = originals_in(root.path())?;
        assert_eq!(inside, ["target"], "{row}");
        assert_eq!(
            fs::read(root.path().join(&originals).join("target"))?,
            b"old"
        );
        assert_eq!(fs::read(&source)?, b"new", "{row}");
        for name in names(root.path())? {
            if name != originals && name != "source" {
                let class = classify(OsStr::new(&name));
                assert_eq!(class, Some(Leftover::IncompleteCopy), "{row}: {name}");
                assert_eq!(fs::read(root.path().join(&name))?, b"new", "{row}: {name}");
            }
        }
    }
    Ok(())
}

#[test]
fn an_unplug_mid_case_only_rename_keeps_the_old_spelling() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let item = root.path().join("report");
    fs::write(&item, b"data")?;
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            case_insensitive: true,
            unplugged_after: Some((Op::Rename, "report", 1)),
            ..Quirks::ext4()
        },
    );
    let parent = open_local_parent_directory(root.path())?;

    let renamed = rename_in_place(
        fs(),
        Entry::new(&parent, OsStr::new("report")),
        Entry::new(&parent, OsStr::new("Report")),
    );

    scope.replug();
    let (originals, inside) = originals_in(root.path())?;
    assert_eq!(names(root.path())?, [originals.as_str()]);
    assert_eq!(inside, ["report"]);
    assert_eq!(
        fs::read(root.path().join(&originals).join("report"))?,
        b"data"
    );
    assert_eq!(
        renamed,
        Err(RenameFailure::Stranded {
            temporary: Path::new(&originals).join("report").into_os_string(),
            error: PlaceError::Unconfirmed,
        })
    );
    Ok(())
}

#[test]
fn an_unplug_after_an_exchange_leaves_no_original_at_a_stage() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"new")?;
    fs::write(&target, b"old")?;
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            trash: Trash::Unsupported,
            unplugged_after: Some((Op::Exchange, "target", 0)),
            ..Quirks::ext4()
        },
    );

    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_err());
    scope.replug();
    let (originals, inside) = originals_in(root.path())?;
    assert_eq!(
        names(root.path())?,
        [originals.as_str(), "source", "target"]
    );
    assert_eq!(inside, ["target"]);
    assert_eq!(
        fs::read(root.path().join(&originals).join("target"))?,
        b"old"
    );
    assert_eq!(fs::read(&target)?, b"new");
    Ok(())
}

#[test]
fn a_refused_exchange_sets_the_original_aside_after_moving_the_copy_back()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"new")?;
    fs::write(&target, b"old")?;
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            trash: Trash::Unsupported,
            fail: vec![(Op::Exchange, "*", 0, Errno::INVAL)],
            unplugged_after: Some((Op::Rename, "target", 2)),
            ..Quirks::ext4()
        },
    );

    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_err());
    assert_eq!(scope.count(Op::Exchange), 1);
    scope.replug();
    let (originals, inside) = originals_in(root.path())?;
    assert_eq!(inside, ["target"]);
    assert_eq!(
        fs::read(root.path().join(&originals).join("target"))?,
        b"old"
    );
    let left = names(root.path())?;
    let [stage, ..] = left.as_slice() else {
        return Err(format!("the copy stays at its stage: {left:?}").into());
    };
    assert_eq!(classify(OsStr::new(stage)), Some(Leftover::IncompleteCopy));
    assert_eq!(fs::read(root.path().join(stage))?, b"new");
    assert_eq!(left, [stage.as_str(), originals.as_str(), "source"]);
    Ok(())
}

fn recover(container: &Path) -> Vec<OperationEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.recover(
        RecoverRequest {
            id: OperationRequestId(1590),
            container: Location::local(container),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_operation(&events, |event| {
        matches!(
            event,
            OperationEvent::Restored { .. }
                | OperationEvent::RestoreCompletedWithErrors { .. }
                | OperationEvent::Failed { .. }
                | OperationEvent::Cancelled { .. }
        )
    });
    events.take()
}

fn recovered_report(events: &[OperationEvent]) -> TransferReport {
    let mut report = TransferReport::default();
    for event in events {
        if let OperationEvent::ItemsReported {
            report: reported, ..
        } = event
        {
            report.extend(reported.clone());
        }
    }
    report
}

fn originals_folder(root: &Path) -> io::Result<PathBuf> {
    let container = root.join(HiddenName::Originals.fresh());
    fs::create_dir(&container)?;
    Ok(container)
}

#[test]
fn restoring_originals_never_replaces_an_item() -> Result<(), Box<dyn Error>> {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Target {
        Free,
        Taken,
        TakenMeanwhile,
        LostReply,
        SameFile,
        FolderTaken,
        ReadOnlyNtfs3,
        EmptyReadOnlyNtfs3,
    }
    let _serial = serial()?;
    for target in [
        Target::Free,
        Target::Taken,
        Target::TakenMeanwhile,
        Target::LostReply,
        Target::SameFile,
        Target::FolderTaken,
        Target::ReadOnlyNtfs3,
        Target::EmptyReadOnlyNtfs3,
    ] {
        let row = format!("{target:?}");
        let root = tempfile::tempdir()?;
        let container = originals_folder(root.path())?;
        let name = if target == Target::FolderTaken {
            fs::create_dir(container.join("album"))?;
            fs::write(container.join("album/song.flac"), b"old")?;
            fs::create_dir(root.path().join("album"))?;
            "album"
        } else {
            if target != Target::EmptyReadOnlyNtfs3 {
                fs::write(container.join("song.flac"), b"old")?;
            }
            "song.flac"
        };
        let read_only = matches!(target, Target::ReadOnlyNtfs3 | Target::EmptyReadOnlyNtfs3);
        match target {
            Target::Taken => fs::write(root.path().join(name), b"new")?,
            Target::SameFile => fs::hard_link(container.join(name), root.path().join(name))?,
            _ => {}
        }
        let racing = root.path().join(name);
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                before: match target {
                    Target::TakenMeanwhile => vec![(
                        Op::Rename,
                        "song.flac",
                        0,
                        Box::new(move || fs::write(&racing, b"racer").expect("the racer's item"))
                            as super::fs_quirks::Racer,
                    )],
                    _ => Vec::new(),
                },
                lost_reply: match target {
                    Target::LostReply => vec![(Op::Rename, "song.flac", Errno::IO)],
                    _ => Vec::new(),
                },
                fail: match target {
                    Target::ReadOnlyNtfs3 => vec![
                        (Op::Rename, "song.flac", 0, Errno::ROFS),
                        (Op::Unlink, ".strata-originals-*", 0, Errno::ROFS),
                    ],
                    Target::EmptyReadOnlyNtfs3 => {
                        vec![(Op::Unlink, ".strata-originals-*", 0, Errno::ROFS)]
                    }
                    _ => Vec::new(),
                },
                ..if read_only {
                    Quirks::ntfs3()
                } else {
                    Quirks::ext4()
                }
            },
        );

        let events = recover(&container);

        let report = recovered_report(&events);
        let numbered = if target == Target::FolderTaken {
            "album (1)"
        } else {
            "song (1).flac"
        };
        let (restored_as, untouched) = match target {
            Target::Taken => (Some(numbered), Some(&b"new"[..])),
            Target::TakenMeanwhile => (Some(numbered), Some(&b"racer"[..])),
            Target::FolderTaken => (Some(numbered), None),
            Target::ReadOnlyNtfs3 | Target::EmptyReadOnlyNtfs3 => (None, None),
            Target::Free | Target::LostReply | Target::SameFile => (Some(name), None),
        };
        let Some(restored_as) = restored_as else {
            let Some(OperationEvent::RestoreCompletedWithErrors { message, .. }) = events.last()
            else {
                return Err(format!("{row}: the restore fails: {events:?}").into());
            };
            let check = crate::i18n::tr(
                "The drive is read-only. It may need to be checked for errors after it was unplugged without being ejected.",
            );
            assert!(message.contains(&check), "{row}: {message}");
            let container_name = container.file_name().ok_or("a name")?.to_string_lossy();
            assert_eq!(
                message.contains(container_name.as_ref()),
                target == Target::EmptyReadOnlyNtfs3,
                "{row}: {message}"
            );
            assert!(report.kept.is_empty(), "{row}: {report:?}");
            let inside: &[&str] = if target == Target::EmptyReadOnlyNtfs3 {
                &[]
            } else {
                &[name]
            };
            assert_eq!(names(&container)?, inside, "{row}");
            continue;
        };
        let restored = root.path().join(restored_as);
        assert!(
            matches!(
                events.last(),
                Some(OperationEvent::Restored { locations, restored: placed, .. })
                    if locations == &[Location::local(&container)]
                        && placed == &[Location::local(&restored)]
            ),
            "{row}: {events:?}"
        );
        let content = if target == Target::FolderTaken {
            restored.join("song.flac")
        } else {
            restored.clone()
        };
        assert_eq!(fs::read(&content)?, b"old", "{row}");
        if let Some(untouched) = untouched {
            assert_eq!(fs::read(root.path().join(name))?, untouched, "{row}");
        }
        let mut expected = vec![name.to_owned()];
        if restored_as != name {
            expected.push(restored_as.to_owned());
            assert_eq!(
                report.renamed,
                [RenamedItem {
                    requested: Location::local(root.path().join(name)),
                    saved_as: Location::local(&restored),
                    cause: RenameCause::Taken,
                }],
                "{row}"
            );
        } else {
            assert!(report.is_empty(), "{row}: {report:?}");
        }
        expected.sort();
        assert_eq!(names(root.path())?, expected, "{row}");
        if target == Target::SameFile {
            assert_eq!(fs::metadata(&restored)?.nlink(), 1, "{row}");
        }
    }
    Ok(())
}

#[test]
fn restoring_a_merge_folder_puts_each_original_in_its_folder() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let container = root.path().join(HiddenName::Merge.fresh());
    for (path, content) in [
        ("a.txt", "old a"),
        ("sub/b.txt", "old b"),
        ("sub/deeper/c.txt", "old c"),
        ("gone/d.txt", "old d"),
    ] {
        let original = container.join(path);
        fs::create_dir_all(original.parent().ok_or("a parent")?)?;
        fs::write(original, content)?;
    }
    fs::write(root.path().join("a.txt"), b"merged a")?;
    fs::create_dir_all(root.path().join("sub/deeper"))?;
    let _scope = FsQuirks::install(root.path(), Quirks::ext4());

    let events = recover(&container);

    let restored: Vec<_> = ["a (1).txt", "sub/b.txt", "sub/deeper/c.txt", "gone/d.txt"]
        .map(|path| Location::local(root.path().join(path)))
        .into_iter()
        .collect();
    let Some(OperationEvent::Restored {
        locations,
        restored: placed,
        ..
    }) = events.last()
    else {
        return Err(format!("the restore completes: {events:?}").into());
    };
    assert_eq!(locations, &[Location::local(&container)]);
    let mut placed = placed.clone();
    placed.sort_by_key(|location| location.native_path().map(Path::to_path_buf));
    let mut expected = restored.clone();
    expected.sort_by_key(|location| location.native_path().map(Path::to_path_buf));
    assert_eq!(placed, expected);
    for (path, content) in [
        ("a.txt", "merged a"),
        ("a (1).txt", "old a"),
        ("sub/b.txt", "old b"),
        ("sub/deeper/c.txt", "old c"),
        ("gone/d.txt", "old d"),
    ] {
        assert_eq!(
            fs::read_to_string(root.path().join(path))?,
            content,
            "{path}"
        );
    }
    assert_eq!(
        recovered_report(&events).renamed,
        [RenamedItem {
            requested: Location::local(root.path().join("a.txt")),
            saved_as: Location::local(root.path().join("a (1).txt")),
            cause: RenameCause::Taken,
        }]
    );
    assert_eq!(names(root.path())?, ["a (1).txt", "a.txt", "gone", "sub"]);
    Ok(())
}

#[test]
fn a_container_holding_only_another_program_s_leftover_is_never_restored()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let program = || Holder::Program("Text Editor".to_owned());
    let rows: [(&str, Preset, HiddenName, &str); 5] = [
        ("nfs3", Quirks::nfs3, HiddenName::Originals, "song.flac"),
        ("sshfs", Quirks::sshfs, HiddenName::Originals, "song.flac"),
        (
            "nfs3",
            Quirks::nfs3,
            HiddenName::Originals,
            "album/song.flac",
        ),
        (
            "cifs-posix",
            Quirks::cifs_posix,
            HiddenName::Originals,
            "song.flac",
        ),
        ("sshfs", Quirks::sshfs, HiddenName::Merge, "sub/song.flac"),
    ];
    for (mount, preset, kind, held) in rows {
        let row = format!("{mount}, {kind:?}, {held}");
        let _clock = ClockGuard::set(SystemTime::now());
        let root = tempfile::tempdir()?;
        let container = root.path().join(kind.fresh());
        fs::create_dir(&container)?;
        let original = container.join(held);
        fs::create_dir_all(original.parent().ok_or("a parent")?)?;
        fs::write(&original, b"old")?;
        let scope = FsQuirks::install(
            root.path(),
            Quirks {
                holder: (mount == "cifs-posix").then(program),
                ..preset()
            },
        );
        let hold = super::fs_quirks::hold(&original);
        let dir = open_local_parent_directory(original.parent().ok_or("a parent")?)?;
        fs().unlink(
            Entry::new(&dir, OsStr::new("song.flac")),
            rustix::fs::AtFlags::empty(),
        )
        .map_err(|error| format!("{row}: {error}"))?;
        let inside = names(&container)?;
        let leftover = names(original.parent().ok_or("a parent")?)?;
        scope.clear_calls();

        let events = recover(&container);

        let report = recovered_report(&events);
        assert!(
            matches!(
                events.last(),
                Some(OperationEvent::Restored { locations, restored, .. })
                    if locations.is_empty() && restored.is_empty()
            ),
            "{row}: {events:?}"
        );
        let holder = (mount == "cifs-posix").then(program);
        assert_eq!(
            report.kept,
            [KeptItem {
                location: Location::local(original.parent().ok_or("a parent")?),
                cause: KeptCause::StillOpen(holder),
            }],
            "{row}"
        );
        assert_eq!(names(&container)?, inside, "{row}");
        let file_name = container.file_name().ok_or("a name")?.to_string_lossy();
        assert_eq!(names(root.path())?, [file_name.as_ref()], "{row}");
        let touched: Vec<_> = scope
            .calls()
            .into_iter()
            .filter(|call| matches!(call.op, Op::Rename | Op::Unlink | Op::Link))
            .filter(|call| {
                call.names
                    .iter()
                    .any(|name| leftover.iter().any(|left| OsStr::new(left) == name))
            })
            .collect();
        assert!(touched.is_empty(), "{row}: {touched:?}");
        drop(hold);
    }
    Ok(())
}

#[test]
fn stale_probes_go_only_when_empty_and_old() -> Result<(), Box<dyn Error>> {
    #[derive(Clone, Copy, Debug)]
    enum Probe {
        File(&'static [u8]),
        Folder { child: bool },
    }
    let _serial = serial()?;
    let hour = Duration::from_secs(60 * 60);
    let uuid = || HiddenName::Probe.fresh().to_string_lossy().into_owned();
    struct Row {
        row: &'static str,
        name: String,
        probe: Probe,
        age: Duration,
        fault: Option<(Op, Errno)>,
        goes: bool,
    }
    let old = |row, probe, goes| Row {
        row,
        name: uuid(),
        probe,
        age: 2 * hour,
        fault: None,
        goes,
    };
    let rows = [
        old("old empty file", Probe::File(b""), true),
        old("old empty folder", Probe::Folder { child: false }, true),
        Row {
            age: Duration::ZERO,
            ..old("new empty file", Probe::File(b""), false)
        },
        old("old file with data", Probe::File(b"data"), false),
        old(
            "old folder with a child",
            Probe::Folder { child: true },
            false,
        ),
        Row {
            name: ".strata-probe-notes".to_owned(),
            ..old("not a probe name", Probe::File(b""), false)
        },
        Row {
            fault: Some((Op::Unlink, Errno::ROFS)),
            ..old("read-only", Probe::File(b""), false)
        },
        Row {
            fault: Some((Op::Stat, Errno::IO)),
            ..old("unplugged", Probe::File(b""), false)
        },
    ];
    for Row {
        row,
        name,
        probe,
        age,
        fault,
        goes,
    } in rows
    {
        let root = tempfile::tempdir()?;
        let probe_path = root.path().join(&name);
        match probe {
            Probe::File(content) => fs::write(&probe_path, content)?,
            Probe::Folder { child } => {
                fs::create_dir(&probe_path)?;
                if child {
                    fs::write(probe_path.join("child"), b"")?;
                }
            }
        }
        fs::File::open(&probe_path)?.set_modified(SystemTime::now() - age)?;
        let pattern: &'static str = Box::leak(name.clone().into_boxed_str());
        let scope = FsQuirks::install(
            root.path(),
            Quirks {
                fail: fault
                    .map(|(op, errno)| (op, pattern, 0, errno))
                    .into_iter()
                    .collect(),
                ..Quirks::ext4()
            },
        );
        let listed = [OsString::from(&name)];
        let unlaned = mount_lanes::Progress::unlaned();

        super::super::leftovers::sweep_probes(fs(), root.path(), &listed, &unlaned);

        assert_eq!(probe_path.exists(), !goes, "{row}");
        scope.clear_calls();
        super::super::leftovers::sweep_probes(fs(), root.path(), &listed, &unlaned);
        assert!(scope.calls().is_empty(), "{row}: {:?}", scope.calls());
    }

    let root = tempfile::tempdir()?;
    let probe = root.path().join(HiddenName::Probe.fresh());
    fs::write(&probe, b"")?;
    fs::File::open(&probe)?.set_modified(SystemTime::now() - 2 * hour)?;
    let slow =
        || -> super::fs_quirks::Racer { Box::new(|| thread::sleep(Duration::from_millis(1200))) };
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![
                (Op::Stat, ".strata-probe-*", 0, slow()),
                (Op::Unlink, ".strata-probe-*", 0, slow()),
            ],
            ..Quirks::ext4()
        },
    );
    LocalOperationProvider.sweep_probes(
        &Location::local(root.path()),
        &[probe.file_name().ok_or("a name")?.to_os_string()],
    );
    let mut states = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while mount_lanes::admit(root.path(), mount_lanes::Class::Background).is_err()
        && Instant::now() < deadline
    {
        states.push(mount_lanes::responsiveness(root.path()));
        glib::MainContext::default().iteration(false);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !probe.exists(),
        "a listed stale probe goes in the background"
    );
    states.push(mount_lanes::responsiveness(root.path()));
    states.dedup();
    assert_eq!(
        states,
        [mount_lanes::Responsiveness::Answering],
        "a slow sweep that keeps answering never stalls its mount"
    );
    Ok(())
}

#[test]
fn a_review_lists_what_each_leftover_holds_up_to_its_bound() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let unreadable = root.path().join(HiddenName::Originals.fresh());
    fs::create_dir(&unreadable)?;
    fs::write(unreadable.join("song.flac"), b"old")?;
    let partly_unreadable = root.path().join(HiddenName::Merge.fresh());
    fs::create_dir_all(partly_unreadable.join("sub"))?;
    fs::write(partly_unreadable.join("sub/c.txt"), b"c")?;
    let originals = root.path().join(HiddenName::Originals.fresh());
    fs::create_dir_all(originals.join("album"))?;
    fs::write(originals.join("song.flac"), b"old")?;
    let merge = root.path().join(HiddenName::Merge.fresh());
    fs::create_dir_all(merge.join("sub/deep"))?;
    fs::write(merge.join("sub/deep/a.txt"), b"a")?;
    fs::write(merge.join("b.txt"), b"b")?;
    let held = root.path().join(HiddenName::Originals.fresh());
    fs::create_dir(&held)?;
    fs::write(held.join("song.flac"), b"old")?;
    let crowded = root.path().join(HiddenName::Originals.fresh());
    fs::create_dir(&crowded)?;
    for index in 0..1_002 {
        crate::test_support::create_empty_file(&crowded.join(format!("{index:04}")))?;
    }
    let empty = root.path().join(HiddenName::Originals.fresh());
    fs::create_dir(&empty)?;
    let stage = root.path().join(HiddenName::Copy.fresh());
    fs::write(&stage, b"partial")?;
    let gone = root.path().join(HiddenName::Replaced.fresh());
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            holder: Some(Holder::Program("Text Editor".to_owned())),
            fail: vec![
                (Op::Names, ".", 0, Errno::IO),
                (Op::Names, ".", 1, Errno::IO),
            ],
            ..Quirks::nfs3()
        },
    );
    let hold = super::fs_quirks::hold(&held.join("song.flac"));
    let dir = open_local_parent_directory(&held)?;
    fs().unlink(
        Entry::new(&dir, OsStr::new("song.flac")),
        rustix::fs::AtFlags::empty(),
    )?;
    let name = |path: &Path| path.file_name().map(OsStr::to_os_string).ok_or("a name");
    let listed = [
        &unreadable,
        &partly_unreadable,
        &originals,
        &merge,
        &held,
        &crowded,
        &empty,
        &stage,
        &gone,
    ]
    .into_iter()
    .map(|path| name(path))
    .collect::<Result<Vec<_>, _>>()?;
    let modified = |path: &Path| fs::symlink_metadata(path).map(|metadata| Some(metadata.mtime()));
    let mut crowded_names: Vec<PathBuf> = (0..1_000)
        .map(|index| format!("{index:04}").into())
        .collect();
    crowded_names.sort();

    let reviewed = super::super::leftovers::review(
        fs(),
        root.path(),
        &listed,
        &mount_lanes::Progress::unlaned(),
    );

    let failed_read = crate::services::io_error_detail(&Errno::IO.into());
    let expected = [
        (&unreadable, LeftoverInside::Unreadable(failed_read.clone())),
        (&partly_unreadable, LeftoverInside::Unreadable(failed_read)),
        (
            &originals,
            LeftoverInside::Items {
                names: vec!["album".into(), "song.flac".into()],
                more: 0,
            },
        ),
        (
            &merge,
            LeftoverInside::Items {
                names: vec!["b.txt".into(), "sub/deep/a.txt".into()],
                more: 0,
            },
        ),
        (
            &held,
            LeftoverInside::StillOpen(
                "A file in it is still open in Text Editor. Close it there and try again."
                    .to_owned(),
            ),
        ),
        (
            &crowded,
            LeftoverInside::Items {
                names: crowded_names,
                more: 2,
            },
        ),
        (
            &empty,
            LeftoverInside::Items {
                names: Vec::new(),
                more: 0,
            },
        ),
        (&stage, LeftoverInside::Item),
    ];
    let mut want = expected
        .into_iter()
        .map(|(path, inside)| {
            Ok(ReviewedLeftover {
                name: name(path)?,
                modified: modified(path)?,
                inside,
            })
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    want.push(ReviewedLeftover {
        name: name(&gone)?,
        modified: None,
        inside: LeftoverInside::Gone,
    });
    assert_eq!(reviewed, want);
    let touched: Vec<_> = scope
        .calls()
        .into_iter()
        .filter(|call| matches!(call.op, Op::Rename | Op::Unlink | Op::Link | Op::Mkdir))
        .collect();
    assert_eq!(touched.len(), 1, "only the fixture's unlink: {touched:?}");

    let answer = Rc::new(RefCell::new(None));
    let answered = answer.clone();
    let _review = LocalOperationProvider.review_leftovers(
        &Location::local(root.path()),
        vec![name(&stage)?],
        Box::new(move |reviewed| {
            answered.replace(Some(reviewed));
        }),
    );
    while answer.borrow().is_none() {
        glib::MainContext::default().iteration(true);
    }
    assert_eq!(
        answer.take(),
        Some(vec![ReviewedLeftover {
            name: name(&stage)?,
            modified: modified(&stage)?,
            inside: LeftoverInside::Item,
        }])
    );
    drop(hold);
    Ok(())
}
