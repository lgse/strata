// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_source_is_deleted_once_its_copy_was_stamped_again_at_close() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // A FUSE writeback cache stamps the close time, within the 2 s of a
    // recent source's that the times check lets stand. An NFSv4.2 write
    // delegation stamps any copy, which gets the source's times again after
    // the close, by name where its owner may neither read nor write it.
    let fuse_writeback = || Quirks {
        times_reset_at_close: true,
        ..Quirks::sshfs()
    };
    let rows: [(&str, Preset, bool, Option<u32>, bool); 4] = [
        ("FUSE writeback cache", fuse_writeback, false, None, true),
        ("NFSv4.2", Quirks::nfs42, false, None, false),
        ("NFSv4.2, inside a folder", Quirks::nfs42, true, None, false),
        (
            "NFSv4.2, unreadable",
            Quirks::nfs42,
            false,
            Some(0o044),
            false,
        ),
    ];
    for (row, preset, in_folder, mode, recent) in rows {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir_all(sources.join("folder"))?;
        fs::create_dir(&destination)?;
        let source = sources.join("folder/notes.txt");
        fs::write(&source, b"notes")?;
        if !recent {
            let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
            set_times(&source, now.as_secs() - 3600, 0)?;
        }
        let _sources = FsQuirks::install(
            &sources,
            Quirks {
                reported_mode: mode.map(|mode| ("notes.txt", mode)),
                ..Quirks::default()
            },
        );
        let _destination = FsQuirks::install(&destination, preset());
        let (item, moved) = if in_folder {
            (sources.join("folder"), destination.join("folder/notes.txt"))
        } else {
            (source.clone(), destination.join("notes.txt"))
        };

        let (_operation, events) =
            paste_move(98, &destination, &item, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        assert!(!item.exists(), "{row}: {:?}", reported(&events).kept);
        fs::set_permissions(&moved, fs::Permissions::from_mode(0o644))?;
        assert_eq!(fs::read(moved)?, b"notes", "{row}");
    }
    Ok(())
}

#[test]
fn a_copy_replaced_before_its_times_are_settled_keeps_its_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // Outside sshfs's 3 s window, so the copy of a recent source is not
    // compared by content.
    let fuse_daemon = || Quirks {
        mount_type: Some(("fuse.rclone", "rw,user_id=1000,group_id=1000")),
        ..Quirks::sshfs()
    };
    let rows: [(&str, Preset, u64, u32); 3] = [
        ("times set again", Quirks::sshfs, 3600, 0o644),
        ("times set again by name", Quirks::sshfs, 3600, 0o044),
        ("times let stand", fuse_daemon, 1, 0o644),
    ];
    for (row, preset, age, mode) in rows {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        let source = sources.join("folder");
        fs::create_dir_all(&source)?;
        fs::create_dir(&destination)?;
        let notes = source.join("notes.txt");
        fs::write(&notes, b"notes")?;
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
        set_times(&notes, now.as_secs() - age, 500_000_000)?;
        let replaced = Arc::new(AtomicBool::new(false));
        let replacing = replaced.clone();
        let staged = destination.clone();
        let racer: Racer = Box::new(move || {
            for entry in fs::read_dir(&staged).expect("list the destination") {
                let copy = entry.expect("an entry").path().join("notes.txt");
                if copy.exists() {
                    fs::write(&copy, b"another program's file").expect("replace the copy");
                    fs::set_permissions(&copy, fs::Permissions::from_mode(mode))
                        .expect("set the replacement's mode");
                    replacing.store(true, Ordering::Release);
                    return;
                }
            }
        });
        let _sources = FsQuirks::install(&sources, Quirks::ext4());
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                after: vec![(Op::Close, "notes.txt", 0, racer)],
                ..preset()
            },
        );

        let (_operation, events) =
            paste_move(98, &destination, &source, TransferConflict::FailIfExists);
        pump_until_transfer(&events);

        assert!(replaced.load(Ordering::Acquire), "{row}");
        assert_eq!(fs::read(&notes).ok(), Some(b"notes".to_vec()), "{row}");
        assert_eq!(
            reported(&events).kept,
            [kept(source, KeptCause::CopyGone)],
            "{row}"
        );
    }
    Ok(())
}

/// A special file the move publishes, whether the mount can place it, and
/// whether another program replaces what it published.
#[derive(Clone, Copy)]
struct Special {
    pipe: bool,
    can_place: bool,
    replace: bool,
}

impl Special {
    fn row(self) -> String {
        let Self {
            pipe,
            can_place,
            replace,
        } = self;
        format!("pipe: {pipe}, can_place: {can_place}, replace: {replace}")
    }

    /// A link made again at its name holds the copy once it points where
    /// the staged link did.
    fn proven(self) -> bool {
        !self.pipe && !self.replace
    }

    fn make(self, source: &Path) -> io::Result<()> {
        if !self.pipe {
            return std::os::unix::fs::symlink("original-target", source);
        }
        rustix::fs::mknodat(
            rustix::fs::CWD,
            source,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::from_raw_mode(0o600),
            0,
        )?;
        Ok(())
    }

    /// After each call that publishes the file, puts `arrival` in its place
    /// once.
    fn replacing(self, arrival: &Path, target: &Path, replaced: &Arc<AtomicBool>) -> Vec<Before> {
        let creation = if self.pipe {
            Op::CreateNode
        } else {
            Op::Symlink
        };
        std::iter::once(creation)
            .chain(self.can_place.then_some(Op::Rename))
            .filter(|_| self.replace)
            .map(|op| {
                let (arrival, target, replaced) = (
                    arrival.to_path_buf(),
                    target.to_path_buf(),
                    replaced.clone(),
                );
                let racer: Racer = Box::new(move || {
                    if !replaced.swap(true, Ordering::AcqRel) {
                        fs::rename(&arrival, &target).expect("replace the published special file");
                    }
                });
                (op, "special", 0, racer)
            })
            .collect()
    }

    fn placement(self) -> Quirks {
        if self.can_place {
            Quirks::default()
        } else {
            Quirks::without_placement()
        }
    }
}

/// What stands at the published name: another program's file, or the copy.
fn assert_published(
    case: Special,
    events: &RefCell<Vec<OperationEvent>>,
    source: &Path,
    published: &Path,
) -> io::Result<()> {
    if case.replace {
        assert_eq!(fs::read(published)?, b"another program's file");
        assert_eq!(fs::metadata(published)?.mode() & 0o777, 0o640);
        return Ok(());
    }
    let expected = if case.proven() {
        Vec::new()
    } else {
        vec![kept(source.to_path_buf(), KeptCause::CopyGone)]
    };
    assert_eq!(reported(events).kept, expected, "{}", case.row());
    if case.pipe {
        assert!(fs::symlink_metadata(published)?.file_type().is_fifo());
    } else {
        assert_eq!(fs::read_link(published)?, PathBuf::from("original-target"));
    }
    Ok(())
}

fn check_special_move(case: Special) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir(&sources)?;
    fs::create_dir(&destination)?;
    let source = sources.join("special");
    case.make(&source)?;
    let arrival = root.path().join("arrival");
    fs::write(&arrival, b"another program's file")?;
    fs::set_permissions(&arrival, fs::Permissions::from_mode(0o640))?;
    let replaced = Arc::new(AtomicBool::new(false));
    let published = destination.join("special");
    let after = case.replacing(&arrival, &published, &replaced);
    let _sources = FsQuirks::install(&sources, Quirks::default());
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            after,
            ..case.placement()
        },
    );
    let (_operation, events) =
        paste_move(99, &destination, &source, TransferConflict::FailIfExists);
    pump_until_transfer(&events);

    let row = case.row();
    assert_eq!(replaced.load(Ordering::Acquire), case.replace, "{row}");
    let proven = case.proven();
    assert_eq!(
        fs::symlink_metadata(&source).is_ok(),
        !proven,
        "{row}: {:?}",
        events.borrow()
    );
    assert_published(case, &events, &source, &published)?;
    if case.pipe {
        assert!(fs::symlink_metadata(&source)?.file_type().is_fifo());
    } else if !proven {
        assert_eq!(fs::read_link(&source)?, PathBuf::from("original-target"));
    }
    Ok(())
}

#[test]
fn a_special_file_move_keeps_its_source_without_proven_published_ownership()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (pipe, can_place, replace) in [
        (false, true, true),
        (true, true, true),
        (false, false, true),
        (true, false, true),
        (false, false, false),
        (true, false, false),
    ] {
        check_special_move(Special {
            pipe,
            can_place,
            replace,
        })?;
    }
    Ok(())
}

#[test]
fn a_replacing_move_keeps_the_actual_copy_out_identity() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    let source = sources.join("folder");
    fs::create_dir_all(&source)?;
    fs::create_dir_all(destination.join("folder"))?;
    fs::write(source.join("notes.txt"), b"notes")?;
    fs::write(destination.join("folder/old.txt"), b"old")?;
    let _sources = FsQuirks::install(&sources, Quirks::default());
    let destination_scope = FsQuirks::install(
        &destination,
        Quirks {
            exchange: super::super::fs_quirks::Exchange::Rejected,
            ..Quirks::vboxsf()
        },
    );
    let (_operation, events) =
        paste_move(98, &destination, &source, TransferConflict::ReplaceExisting);
    pump_until_transfer(&events);
    assert_pasted(&events, "");
    assert_eq!(fs::read(destination.join("folder/notes.txt"))?, b"notes");
    let calls = destination_scope.calls();
    assert!(
        calls
            .iter()
            .any(|call| call.op == Op::Mkdir && call.names.iter().any(|name| name == "folder")),
        "exclusive copy-out created the final folder"
    );
    let report = reported(&events);
    assert_eq!(
        (source.exists(), report.kept.is_empty()),
        (false, true),
        "{report:?}"
    );
    Ok(())
}
