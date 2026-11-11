// SPDX-License-Identifier: MIT

use super::*;

/// What udev records the device as holding; an error is a lookup that fails
/// on an NTFS device.
type Device = Option<Result<&'static str, Errno>>;

/// A destination family and what its barrier before deleting a moved
/// source does there.
struct Family {
    fs_type: &'static str,
    device: Device,
    kernel: Option<(u32, u32, u32)>,
    removable: bool,
    moving: bool,
    /// Whether each written file is synced before it is closed.
    each_file: bool,
    barrier: &'static [Op],
}

impl Family {
    fn row(&self) -> String {
        let Self {
            fs_type,
            device,
            kernel,
            removable,
            moving,
            ..
        } = self;
        format!(
            "{fs_type} holding {device:?} on {kernel:?} removable: {removable} moving: {moving}"
        )
    }

    fn quirks(&self) -> Quirks {
        Quirks {
            mount_type: Some((self.fs_type, "rw")),
            device_format: self.device.map(|format| format.unwrap_or("ntfs")),
            fail: match self.device {
                Some(Err(error)) => vec![(Op::DeviceFormat, "*", 0, error)],
                _ => Vec::new(),
            },
            ..Quirks::default()
        }
    }
}

const fn family(
    fs_type: &'static str,
    device: Device,
    removable: bool,
    each_file: bool,
    barrier: &'static [Op],
) -> Family {
    Family {
        fs_type,
        device,
        kernel: None,
        removable,
        moving: true,
        each_file,
        barrier,
    }
}

fn check_family(id: u64, family: &Family) -> Result<(), Box<dyn Error>> {
    let row = family.row();
    let _kernel = KernelGuard::set(family.kernel);
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let folder = sources.join("folder");
    fs::create_dir_all(&folder)?;
    fs::write(folder.join("a.txt"), b"a")?;
    fs::write(folder.join("b.txt"), b"bb")?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let _removable = family
        .removable
        .then(|| RemovableFlushGuard::install(&destination, None));
    let destination_quirks = FsQuirks::install(&destination, family.quirks());
    let (_sources, at_first_unlink) =
        CallsAtFirstUnlink::install(&sources, destination_quirks.reader());

    let items = vec![item(&folder, TransferConflict::FailIfExists)];
    let (_operation, events) = paste_pumped(request(id, &destination, items, family.moving));

    assert_pasted(&events, &row);
    assert_eq!(fs::read(destination.join("folder/b.txt"))?, b"bb", "{row}");
    assert_eq!(folder.exists(), !family.moving, "{row}");
    let flushing = events
        .iter()
        .any(|event| matches!(event, OperationEvent::FlushingToDevice { .. }));
    assert_eq!(
        flushing,
        !family.barrier.is_empty(),
        "{row}: the device flush shows"
    );
    let calls = if family.moving {
        at_first_unlink.take()?
    } else {
        destination_quirks.calls()
    };
    assert_each_file_synced(&calls, family.each_file, &row);
    assert_eq!(barrier_ops(&calls), family.barrier, "{row}");
    Ok(())
}

/// Whether each of `a.txt` and `b.txt` is synced, and then before it is closed.
fn assert_each_file_synced(calls: &[Call], each_file: bool, row: &str) {
    let position = |op: Op, name: &str| {
        calls
            .iter()
            .position(|call| call.op == op && call.names.iter().any(|named| named == name))
    };
    for name in ["a.txt", "b.txt"] {
        let synced = position(Op::SyncData, name);
        assert_eq!(synced.is_some(), each_file, "{row}: {name}");
        let closed_after = |synced| position(Op::Close, name).is_some_and(|closed| synced < closed);
        assert!(
            synced.is_none_or(closed_after),
            "{row}: {name} is synced before it is closed"
        );
    }
}

fn barrier_ops(calls: &[Call]) -> Vec<Op> {
    calls
        .iter()
        .map(|call| call.op)
        .filter(|op| matches!(op, Op::SyncFs | Op::SyncDir))
        .collect()
}

#[test]
fn the_barrier_before_deleting_a_moved_source_follows_the_destination_family()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    const NTFS: Device = Some(Ok("ntfs"));
    const EXFAT: Device = Some(Ok("exfat"));
    const HFS: Device = Some(Ok("hfsplus"));
    const FAILS: Device = Some(Err(Errno::ACCESS));
    const FS: &[Op] = &[Op::SyncFs];
    const DIR: &[Op] = &[Op::SyncDir];
    const BOTH: &[Op] = &[Op::SyncDir, Op::SyncFs, Op::SyncDir];
    const FS_THEN_DIR: &[Op] = &[Op::SyncFs, Op::SyncDir];
    let rows = [
        family("ext4", None, false, false, FS),
        family("vfat", None, false, false, BOTH),
        family("ntfs3", None, false, false, BOTH),
        family("fuseblk", None, false, true, DIR),
        family("fuseblk", NTFS, false, false, BOTH),
        family("fuseblk", EXFAT, false, false, BOTH),
        family("fuseblk", HFS, false, true, DIR),
        family("fuseblk", FAILS, false, true, DIR),
        family("cifs", None, false, true, &[]),
        family("9p", None, false, true, FS),
        family("ext4", None, true, false, FS_THEN_DIR),
        Family {
            moving: false,
            ..family("cifs", None, false, false, &[])
        },
        Family {
            kernel: Some((5, 4, 0)),
            ..family("ext4", None, false, true, FS)
        },
    ];
    for (id, family) in (300..).zip(&rows) {
        check_family(id, family)?;
    }
    Ok(())
}

#[test]
fn a_merging_move_flushes_each_filesystem_it_wrote_to_before_deleting() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir_all(sources.join("media/sub"))?;
    fs::create_dir_all(destination.join("media/sub"))?;
    fs::write(sources.join("media/sub/new.bin"), b"new")?;
    let outer = FsQuirks::install(&destination, Quirks::ext4());
    let nested = FsQuirks::install(&destination.join("media/sub"), Quirks::ext4());
    let (_sources, at_first_unlink) = CallsAtFirstUnlink::install(&sources, nested.reader());

    let items = vec![item(&sources.join("media"), TransferConflict::Merge)];
    let (_operation, events) = paste_pumped(request(360, &destination, items, true));

    assert_pasted(&events, "");
    assert_eq!(fs::read(destination.join("media/sub/new.bin"))?, b"new");
    assert!(!sources.join("media").exists());
    let calls: Vec<Op> = at_first_unlink.take()?.iter().map(|call| call.op).collect();
    let opened = calls.iter().position(|op| *op == Op::OpenDir);
    let written = calls
        .iter()
        .position(|op| matches!(op, Op::Write | Op::CopyRange | Op::Clone));
    assert!(
        opened.is_some_and(|opened| written.is_some_and(|written| opened < written)),
        "the nested filesystem's barrier is opened before the copy writes there: {calls:?}"
    );
    assert!(calls.contains(&Op::SyncFs), "{calls:?}");
    assert!(outer.calls().iter().any(|call| call.op == Op::SyncFs));
    Ok(())
}

#[test]
fn files_gio_wrote_are_synced_one_by_one_where_the_barrier_misses_them()
-> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    let folder = destination.join("folder");
    fs::create_dir_all(folder.join("sub"))?;
    fs::write(folder.join("a.txt"), b"a")?;
    fs::write(folder.join("sub/b.txt"), b"b")?;
    let read_only = folder.join("read-only.txt");
    fs::write(&read_only, b"c")?;
    fs::set_permissions(&read_only, fs::Permissions::from_mode(0o444))?;
    fs::write(destination.join("other.txt"), b"not written")?;
    let scope = FsQuirks::install(
        &destination,
        Quirks {
            mount_type: Some(("cifs", "rw")),
            ..Quirks::default()
        },
    );

    glib::MainContext::default().block_on(super::super::super::copy_engine::sync_files(
        vec![folder.clone()],
        &gio::Cancellable::new(),
    ))?;

    let mut synced: Vec<String> = scope
        .calls()
        .iter()
        .filter(|call| call.op == Op::SyncData)
        .flat_map(|call| {
            call.names
                .iter()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .collect();
    synced.sort();
    assert_eq!(synced, ["a.txt", "b.txt", "read-only.txt"]);
    Ok(())
}

fn check_smb_replace(id: u64, trash: Trash, synced: bool) -> Result<(), Box<dyn Error>> {
    let row = format!("{trash:?}");
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir_all(sources.join("folder"))?;
    fs::write(sources.join("folder/a.txt"), b"a")?;
    fs::write(sources.join("folder/b.txt"), b"b")?;
    fs::create_dir_all(destination.join("folder"))?;
    fs::write(destination.join("folder/old.txt"), b"old")?;
    let _sources = FsQuirks::install(&sources, Quirks::default());
    let scope = FsQuirks::install(
        &destination,
        Quirks {
            mount_type: Some(("cifs", "rw")),
            fs_type: Some(0xFF53_4D42),
            trash,
            ..Quirks::default()
        },
    );

    let items = vec![item(
        &sources.join("folder"),
        TransferConflict::ReplaceExisting,
    )];
    let events = run_paste(request(id, &destination, items, false));

    assert_pasted(&events, &row);
    assert_eq!(names(&destination.join("folder"))?, ["a.txt", "b.txt"]);
    let calls = scope.calls();
    let synced_at: Vec<usize> = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| call.op == Op::SyncData)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(synced_at.len(), if synced { 2 } else { 0 }, "{row}");
    let displaced = calls.iter().position(|call| {
        call.op == Op::Rename && call.names.first().is_some_and(|name| name == "folder")
    });
    assert!(
        synced_at
            .iter()
            .all(|synced| displaced.is_some_and(|displaced| *synced < displaced)),
        "{row}: the copy is synced before the original is displaced"
    );
    Ok(())
}

#[test]
fn a_replace_on_smb_syncs_its_copy_only_where_the_original_is_deleted() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    for (id, trash, synced) in [(380, Trash::Supported, false), (381, Trash::Refused, true)] {
        check_smb_replace(id, trash, synced)?;
    }
    Ok(())
}

/// A move of `a.txt` into an unreadable destination on `fs_type`, the
/// destination being the mount's root or a folder `beneath` it.
fn check_unreadable_destination(
    id: u64,
    fs_type: &'static str,
    beneath: bool,
    moved: bool,
) -> Result<(), Box<dyn Error>> {
    let row = format!("{fs_type}, beneath the root: {beneath}");
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let mount = root.path().join("mount");
    let destination = if beneath {
        mount.join("drop")
    } else {
        mount.clone()
    };
    fs::create_dir_all(&sources)?;
    fs::create_dir_all(&destination)?;
    let source = sources.join("a.txt");
    fs::write(&source, b"a")?;
    let _sources = FsQuirks::install(&sources, Quirks::default());
    let scope = FsQuirks::install(
        &mount,
        Quirks {
            mount_type: Some((fs_type, "rw")),
            fail: vec![(Op::OpenDir, ".", 0, Errno::ACCESS)],
            ..Quirks::default()
        },
    );

    let moving = paste_moves(
        Moving {
            id,
            destination: &destination,
            sources: std::slice::from_ref(&source),
            cancel_after: None,
        },
        &Barriers::new(),
    );

    if moved {
        assert_pasted(&moving.events(), &row);
        assert!(!source.exists(), "{row}");
        assert_flushed_names(&scope.calls(), fs_type, beneath, &row);
    } else {
        assert!(
            matches!(terminal_transfer(&moving.events()),
                Some(OperationEvent::TransferFailed { message, .. })
                if message.contains("Could not safely open")),
            "{row}: {:?}",
            moving.events()
        );
        assert!(source.exists(), "{row}");
        assert!(!destination.join("a.txt").exists(), "{row}");
    }
    Ok(())
}

fn assert_flushed_names(calls: &[Call], fs_type: &str, beneath: bool, row: &str) {
    let flushed: Vec<_> = calls
        .iter()
        .filter(|call| matches!(call.op, Op::SyncFs | Op::SyncDir))
        .map(|call| call.names.clone())
        .collect();
    match (fs_type, beneath) {
        ("ext4", true) => assert_eq!(flushed, [["mount"]], "{row}"),
        // The unnamed file made in the unreadable root, shown as deleted.
        ("ext4", false) => assert!(
            flushed.len() == 1
                && flushed[0]
                    .iter()
                    .all(|name| name.to_string_lossy().ends_with("(deleted)")),
            "{row}: {flushed:?}"
        ),
        _ => assert!(flushed.is_empty(), "{row}"),
    }
}

#[test]
fn a_barrier_needs_a_readable_folder_only_where_it_flushes_one() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // The family, whether the unreadable destination is a folder beneath
    // the mount's root, and whether the move goes ahead: `syncfs` takes an
    // unnamed file made in an unreadable root, a folder's `fsync` does not.
    let rows = [
        ("ext4", true, true),
        ("ext4", false, true),
        ("vfat", false, false),
        ("cifs", false, true),
    ];
    for (id, (fs_type, beneath, moved)) in (390..).zip(rows) {
        // Only an unnamed file can flush an unreadable root.
        let flushes_the_root = fs_type == "ext4" && !beneath;
        if flushes_the_root && !base_offers(&UNNAMED_FILES)? {
            continue;
        }
        check_unreadable_destination(id, fs_type, beneath, moved)?;
    }
    Ok(())
}

/// A source at `sources` that `conflict` meets at `destination`.
fn conflicting_source(
    sources: &Path,
    destination: &Path,
    conflict: TransferConflict,
) -> io::Result<PathBuf> {
    if conflict == TransferConflict::Merge {
        fs::create_dir(sources.join("folder"))?;
        fs::write(sources.join("folder/incoming.txt"), b"incoming")?;
        fs::create_dir(destination.join("folder"))?;
        return Ok(sources.join("folder"));
    }
    fs::write(sources.join("incoming.txt"), b"incoming")?;
    if conflict == TransferConflict::ReplaceExisting {
        fs::write(destination.join("incoming.txt"), b"old")?;
    }
    Ok(sources.join("incoming.txt"))
}

fn check_smb_move(id: u64, conflict: TransferConflict) -> Result<(), Box<dyn Error>> {
    let row = format!("{conflict:?}");
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir_all(&sources)?;
    fs::create_dir_all(&destination)?;
    let source = conflicting_source(&sources, &destination, conflict)?;
    let destination_quirks = FsQuirks::install(
        &destination,
        Quirks {
            mount_type: Some(("cifs", "rw")),
            trash: Trash::Unsupported,
            ..Quirks::default()
        },
    );
    let (_sources, at_first_unlink) =
        CallsAtFirstUnlink::install(&sources, destination_quirks.reader());

    let (_operation, events) = paste_pumped(request(
        id,
        &destination,
        vec![item(&source, conflict)],
        true,
    ));

    assert_pasted(&events, &row);
    assert!(!source.exists(), "{row}");
    let calls = at_first_unlink.take()?;
    assert!(
        calls.iter().any(|call| call.op == Op::SyncData),
        "{row}: the copy is synced before its source is deleted: {:?}",
        calls.iter().map(|call| call.op).collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn moved_files_are_synced_on_smb_whatever_the_conflict() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let conflicts = [
        TransferConflict::FailIfExists,
        TransferConflict::ReplaceExisting,
        TransferConflict::Merge,
    ];
    for (id, conflict) in (370..).zip(conflicts) {
        check_smb_move(id, conflict)?;
    }
    Ok(())
}

type Lose = Box<dyn Fn() + Send + Sync>;

/// A racer after `published` that runs the loss once it is armed.
fn armed_loss(published: Op) -> (Arc<std::sync::Mutex<Option<Lose>>>, Before) {
    let loser: Arc<std::sync::Mutex<Option<Lose>>> = Arc::default();
    let lose = loser.clone();
    let racer: Racer = Box::new(move || {
        if let Some(lose) = &*lose.lock().unwrap_or_else(|error| error.into_inner()) {
            lose();
        }
    });
    (loser, (published, "*", 0, racer))
}

fn check_lost_writeback(
    id: u64,
    (published, tmpfile): (Op, Support),
    names: &[&str],
) -> Result<(), Box<dyn Error>> {
    let context = format!("{names:?}, published by {published:?}");
    let root = tempfile::tempdir()?;
    let layout = Layout::create(root.path(), names)?;
    layout.write_sources(|name| name.as_bytes().to_vec())?;
    let _sources = FsQuirks::install(&layout.sources, Quirks::default());
    let (loser, after) = armed_loss(published);
    let scope = FsQuirks::install(
        &layout.destination,
        Quirks {
            tmpfile,
            after: vec![after],
            ..Quirks::ext4()
        },
    );
    *loser.lock().unwrap_or_else(|error| error.into_inner()) =
        Some(Box::new(scope.writeback_loser()));

    let moved = layout.move_all(id, &Barriers::new());

    assert!(
        matches!(terminal_transfer(&moved.events()),
            Some(OperationEvent::TransferFailed { message, .. })
            if message.contains("writes may still be pending")),
        "{context}: {:?}",
        moved.events()
    );
    assert!(layout.paths().iter().all(|path| path.exists()), "{context}");
    moved.assert_moved_after_deletion(0, &context);
    Ok(())
}

#[test]
fn a_writeback_lost_during_a_move_keeps_every_source_it_may_hold() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let _limits = LimitsGuard::set(NO_LIMITS);
    // The first copy's writeback is lost once it is published, with one
    // item, or with a second whose barrier is opened after the loss. A file
    // is published by linking its unnamed copy, or by renaming its named stage.
    let stages = [
        (Op::LinkFd, Support::Supported),
        (Op::Rename, Support::Rejected(Errno::OPNOTSUPP)),
    ];
    let cases = stages
        .into_iter()
        .flat_map(|stage| [&["a"][..], &["a", "b"][..]].map(|names| (stage, names)));
    for (id, (stage, names)) in (440..).zip(cases) {
        if stage.0 == Op::LinkFd && !base_offers(&UNNAMED_FILES)? {
            continue;
        }
        check_lost_writeback(id, stage, names)?;
    }
    Ok(())
}
