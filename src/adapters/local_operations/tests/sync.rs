// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use rustix::io::Errno;

use super::super::{MountFlushHint, SyncDestination, removable_sync_roots, sync_filesystem};
use super::fs_quirks::{Op, Support};
use super::*;

fn hint(root: &str, removable: bool) -> MountFlushHint {
    MountFlushHint {
        root: PathBuf::from(root),
        removable,
    }
}

#[test]
fn removable_copy_roots_are_synced_once() {
    let hints = [
        hint("/media/usb", true),
        hint("/home", false),
        hint("/", false),
    ];

    let removable = removable_sync_roots(
        &[SyncDestination::Local(Path::new("/media/usb/photos/a.jpg"))],
        &hints,
    );
    assert_eq!(removable, vec![PathBuf::from("/media/usb")]);

    let internal = removable_sync_roots(
        &[SyncDestination::Local(Path::new("/home/user/a.jpg"))],
        &hints,
    );
    assert!(internal.is_empty());

    let once = removable_sync_roots(
        &[
            SyncDestination::Local(Path::new("/media/usb/a.jpg")),
            SyncDestination::Local(Path::new("/media/usb/nested/b.jpg")),
        ],
        &hints,
    );
    assert_eq!(once, vec![PathBuf::from("/media/usb")]);

    let unknown = removable_sync_roots(
        &[SyncDestination::Local(Path::new("/var/tmp/file"))],
        &[hint("/media/usb", true)],
    );
    assert!(unknown.is_empty());

    let remote = removable_sync_roots(&[SyncDestination::NonLocal], &hints);
    assert!(remote.is_empty());
}

#[test]
fn sync_filesystem_on_a_local_directory_succeeds() {
    let directory = tempfile::tempdir().expect("temp directory");
    sync_filesystem(directory.path()).expect("syncfs on a local directory");
}

#[test]
fn bytes_already_written_are_flushed_before_cancellation_and_cancel_is_not_pasted()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("usb");
    fs::create_dir(&destination)?;
    let source = root.path().join("photo.bin");
    fs::write(&source, b"pixels")?;
    let _guard = RemovableFlushGuard::install(root.path(), None);
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let handle = Rc::new(RefCell::new(None));
    let slot = handle.clone();
    let operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(99),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            }],
            move_sources: false,
        },
        Rc::new(move |event| {
            let cancel = matches!(
                &event,
                OperationEvent::TransferProgress {
                    completed_items, ..
                } if *completed_items >= 1
            );
            emitted.borrow_mut().push(event);
            if cancel {
                slot.borrow_mut().take();
            }
        }),
    );
    *handle.borrow_mut() = Some(operation);
    pump_until_transfer(&events);

    assert!(matches!(
        terminal_transfer(&events.borrow()),
        Some(OperationEvent::Cancelled { .. })
    ));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, OperationEvent::Pasted { .. }))
    );
    assert_eq!(fs::read(destination.join("photo.bin"))?, b"pixels");
    assert!(
        !sync_probe_observations().is_empty(),
        "written bytes are synced before cancellation is reported"
    );
    Ok(())
}

#[test]
fn a_cancel_after_the_flush_starts_does_not_report_success() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("usb");
    fs::create_dir(&destination)?;
    let source = root.path().join("photo.bin");
    fs::write(&source, b"pixels")?;
    let _guard = RemovableFlushGuard::install(root.path(), None);
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let handle = Rc::new(RefCell::new(None));
    let slot = handle.clone();
    let operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(100),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            }],
            move_sources: false,
        },
        Rc::new(move |event| {
            let cancel = matches!(event, OperationEvent::FlushingToDevice { .. });
            emitted.borrow_mut().push(event);
            if cancel {
                slot.borrow_mut().take();
            }
        }),
    );
    *handle.borrow_mut() = Some(operation);
    pump_until_transfer(&events);

    assert!(matches!(
        terminal_transfer(&events.borrow()),
        Some(OperationEvent::Cancelled { .. })
    ));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, OperationEvent::Pasted { .. }))
    );
    assert_eq!(fs::read(destination.join("photo.bin"))?, b"pixels");
    assert!(!sync_probe_observations().is_empty());
    Ok(())
}

#[test]
fn failed_final_flush_reports_that_writes_may_still_be_pending() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("usb");
    fs::create_dir(&destination)?;
    let source = root.path().join("photo.bin");
    fs::write(&source, b"pixels")?;
    let _guard = RemovableFlushGuard::install(root.path(), Some(io::ErrorKind::Other));
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(106),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            }],
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    pump_until_transfer(&events);
    assert!(matches!(terminal_transfer(&events.borrow()),
        Some(OperationEvent::TransferFailed { message, .. })
        if message.contains("writes may still be pending") && message.contains("wait for safe eject")));
    Ok(())
}

#[test]
fn failed_flush_after_cancel_reports_that_writes_may_still_be_pending() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("usb");
    fs::create_dir(&destination)?;
    let _guard = RemovableFlushGuard::install(root.path(), Some(io::ErrorKind::Other));
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let emit: Rc<dyn Fn(OperationEvent)> = Rc::new(move |event| emitted.borrow_mut().push(event));
    glib::MainContext::default().block_on(stop_transfer(
        &[destination],
        &emit,
        OperationRequestId(105),
        TransferStop {
            completed: Vec::new(),
            failed: Vec::new(),
            not_attempted: Vec::new(),
            affected_locations: HashSet::new(),
            failure: None,
            skip_flush: false,
        },
    ));
    assert!(
        matches!(events.borrow().last(), Some(OperationEvent::TransferFailed { message, .. })
        if message.contains("writes may still be pending") && message.contains("wait for safe eject"))
    );
    assert!(!sync_probe_observations().is_empty());
    Ok(())
}

#[test]
fn fat32_limit_failure_does_not_start_an_uncancellable_device_flush() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("usb");
    fs::create_dir(&destination)?;
    let _guard = RemovableFlushGuard::install(root.path(), None);
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let emit: Rc<dyn Fn(OperationEvent)> = Rc::new(move |event| emitted.borrow_mut().push(event));
    glib::MainContext::default().block_on(stop_transfer(
        &[destination],
        &emit,
        OperationRequestId(104),
        TransferStop {
            completed: Vec::new(),
            failed: Vec::new(),
            not_attempted: Vec::new(),
            affected_locations: HashSet::new(),
            failure: Some("A file exceeds the FAT32 limit".to_owned()),
            skip_flush: true,
        },
    ));
    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::TransferFailed { .. })
    ));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, OperationEvent::FlushingToDevice { .. }))
    );
    assert!(sync_probe_observations().is_empty());
    Ok(())
}

#[test]
fn bytes_already_written_are_flushed_before_transfer_failure() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("usb");
    fs::create_dir(&destination)?;
    let source = root.path().join("photo.bin");
    fs::write(&source, b"pixels")?;
    let _guard = RemovableFlushGuard::install(root.path(), None);
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(101),
            destination: Location::local(&destination),
            items: vec![
                PasteItem {
                    source: Location::local(&source),
                    conflict: TransferConflict::FailIfExists,
                    target_name: None,
                },
                PasteItem {
                    source: Location::local(root.path().join("missing.bin")),
                    conflict: TransferConflict::FailIfExists,
                    target_name: None,
                },
            ],
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    pump_until_transfer(&events);

    assert!(matches!(
        terminal_transfer(&events.borrow()),
        Some(OperationEvent::TransferFailed { .. })
    ));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, OperationEvent::Pasted { .. }))
    );
    assert_eq!(fs::read(destination.join("photo.bin"))?, b"pixels");
    assert!(!sync_probe_observations().is_empty());
    Ok(())
}

#[derive(Debug, PartialEq)]
struct Seen {
    deleted: Vec<String>,
    copied: Vec<String>,
    flushing: bool,
    counted: usize,
}

struct Barriers {
    seen: Arc<std::sync::Mutex<Vec<Seen>>>,
    flushing: Arc<std::sync::atomic::AtomicBool>,
    counted: Arc<std::sync::atomic::AtomicUsize>,
}

impl Barriers {
    fn new() -> Self {
        Self {
            seen: Arc::default(),
            flushing: Arc::default(),
            counted: Arc::default(),
        }
    }

    fn racers(
        &self,
        sources: &Path,
        names: &[&str],
        destination: &Path,
        mut also: Option<Box<dyn FnMut() + Send>>,
    ) -> Vec<(Op, &'static str, usize, super::fs_quirks::Racer)> {
        (0..names.len())
            .map(|_| {
                let (seen, flushing, counted) = (
                    self.seen.clone(),
                    self.flushing.clone(),
                    self.counted.clone(),
                );
                let (sources, destination) = (sources.to_path_buf(), destination.to_path_buf());
                let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
                let mut also = also.take();
                let racer: super::fs_quirks::Racer = Box::new(move || {
                    if let Some(also) = &mut also {
                        also();
                    }
                    let deleted = names
                        .iter()
                        .filter(|name| !sources.join(name).exists())
                        .cloned()
                        .collect();
                    let copied = names
                        .iter()
                        .filter(|name| destination.join(name).exists())
                        .cloned()
                        .collect();
                    seen.lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .push(Seen {
                            deleted,
                            copied,
                            flushing: flushing.load(std::sync::atomic::Ordering::SeqCst),
                            counted: counted.load(std::sync::atomic::Ordering::SeqCst),
                        });
                });
                (Op::SyncFs, "*", 0, racer)
            })
            .collect()
    }

    fn seen(&self) -> Vec<Seen> {
        std::mem::take(&mut *self.seen.lock().unwrap_or_else(|error| error.into_inner()))
    }
}

type PasteMoves = (
    Rc<RefCell<Option<LoadHandle>>>,
    Rc<RefCell<Vec<OperationEvent>>>,
    Rc<RefCell<Vec<Location>>>,
);

fn paste_moves(
    id: u64,
    destination: &Path,
    sources: &[PathBuf],
    barriers: &Barriers,
    cancel_after: Option<usize>,
) -> PasteMoves {
    let events = Rc::new(RefCell::new(Vec::new()));
    let early = Rc::new(RefCell::new(Vec::new()));
    let handle = Rc::new(RefCell::new(None));
    let (emitted, reported_early, slot) = (events.clone(), early.clone(), handle.clone());
    let (flushing, counted) = (barriers.flushing.clone(), barriers.counted.clone());
    let operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(id),
            destination: Location::local(destination),
            items: sources
                .iter()
                .map(|source| PasteItem {
                    source: Location::local(source),
                    conflict: TransferConflict::FailIfExists,
                    target_name: None,
                })
                .collect(),
            move_sources: true,
        },
        Rc::new(move |event| {
            match &event {
                OperationEvent::FlushingToDevice { .. } => {
                    flushing.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                OperationEvent::ItemMoved { from, .. }
                    if from.native_path().is_some_and(Path::exists) =>
                {
                    reported_early.borrow_mut().push(from.clone());
                }
                OperationEvent::TransferProgress {
                    completed_items, ..
                } => {
                    counted.store(*completed_items, std::sync::atomic::Ordering::SeqCst);
                    if Some(*completed_items) == cancel_after {
                        slot.borrow_mut().take();
                    }
                }
                _ => {}
            }
            emitted.borrow_mut().push(event);
        }),
    );
    *handle.borrow_mut() = Some(operation);
    (handle, events, early)
}

fn moved_count(events: &[OperationEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, OperationEvent::ItemMoved { .. }))
        .count()
}

struct LimitsGuard;

impl LimitsGuard {
    fn set(limits: super::super::owed::Limits) -> Self {
        super::super::owed::set_batch_limits_for_test(Some(limits));
        Self
    }
}

impl Drop for LimitsGuard {
    fn drop(&mut self) {
        super::super::owed::set_batch_limits_for_test(None);
    }
}

const NO_LIMITS: super::super::owed::Limits = super::super::owed::Limits {
    items: usize::MAX,
    bytes: u64::MAX,
    age: Duration::MAX,
};

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn the_barrier_before_deleting_a_moved_source_follows_the_destination_family()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // The family, what udev records the device as holding (an error is a
    // lookup that fails on an NTFS device), the kernel, whether the device
    // is removable, whether the paste moves, whether each written file is
    // synced before it is closed, and the barrier's calls.
    type Device = Option<Result<&'static str, Errno>>;
    const NTFS: Device = Some(Ok("ntfs"));
    const EXFAT: Device = Some(Ok("exfat"));
    const HFS: Device = Some(Ok("hfsplus"));
    const FAILS: Device = Some(Err(Errno::ACCESS));
    const FS: &[Op] = &[Op::SyncFs];
    const DIR: &[Op] = &[Op::SyncDir];
    const BOTH: &[Op] = &[Op::SyncDir, Op::SyncFs, Op::SyncDir];
    const FS_THEN_DIR: &[Op] = &[Op::SyncFs, Op::SyncDir];
    type Row = (
        &'static str,
        Device,
        Option<(u32, u32, u32)>,
        bool,
        bool,
        bool,
        &'static [Op],
    );
    let rows: [Row; 13] = [
        ("ext4", None, None, false, true, false, FS),
        ("vfat", None, None, false, true, false, BOTH),
        ("ntfs3", None, None, false, true, false, BOTH),
        ("fuseblk", None, None, false, true, true, DIR),
        ("fuseblk", NTFS, None, false, true, false, BOTH),
        ("fuseblk", EXFAT, None, false, true, false, BOTH),
        ("fuseblk", HFS, None, false, true, true, DIR),
        ("fuseblk", FAILS, None, false, true, true, DIR),
        ("cifs", None, None, false, true, true, &[]),
        ("9p", None, None, false, true, true, FS),
        ("ext4", None, None, true, true, false, FS_THEN_DIR),
        ("cifs", None, None, false, false, false, &[]),
        ("ext4", None, Some((5, 4, 0)), false, true, true, FS),
    ];
    for (id, (fs_type, device, kernel, removable, moving, each_file, barrier)) in (300..).zip(rows)
    {
        let row = format!(
            "{fs_type} holding {device:?} on {kernel:?} removable: {removable} moving: {moving}"
        );
        let _kernel = KernelGuard::set(kernel);
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let folder = sources.join("folder");
        fs::create_dir_all(&folder)?;
        fs::write(folder.join("a.txt"), b"a")?;
        fs::write(folder.join("b.txt"), b"bb")?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let _removable = removable.then(|| RemovableFlushGuard::install(&destination, None));
        let destination_quirks = FsQuirks::install(
            &destination,
            Quirks {
                mount_type: Some((fs_type, "rw")),
                device_format: device.map(|format| format.unwrap_or("ntfs")),
                fail: match device {
                    Some(Err(error)) => vec![(Op::DeviceFormat, "*", 0, error)],
                    _ => Vec::new(),
                },
                ..Quirks::default()
            },
        );
        let at_first_unlink = Arc::new(std::sync::Mutex::new(None));
        let (record, read) = (at_first_unlink.clone(), destination_quirks.reader());
        let _sources = FsQuirks::install(
            &sources,
            Quirks {
                before: vec![(
                    Op::Unlink,
                    "*",
                    0,
                    Box::new(move || {
                        *record.lock().unwrap_or_else(|error| error.into_inner()) = Some(read());
                    }),
                )],
                ..Quirks::default()
            },
        );

        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let _operation = LocalOperationProvider.paste(
            PasteRequest {
                id: OperationRequestId(id),
                destination: Location::local(&destination),
                items: vec![PasteItem {
                    source: Location::local(&folder),
                    conflict: TransferConflict::FailIfExists,
                    target_name: None,
                }],
                move_sources: moving,
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{row}: {:?}",
            events.borrow()
        );
        assert_eq!(fs::read(destination.join("folder/b.txt"))?, b"bb", "{row}");
        assert_eq!(folder.exists(), !moving, "{row}");
        let flushing = events
            .borrow()
            .iter()
            .any(|event| matches!(event, OperationEvent::FlushingToDevice { .. }));
        assert_eq!(
            flushing,
            !barrier.is_empty(),
            "{row}: the device flush shows"
        );
        let calls = if moving {
            at_first_unlink
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
                .ok_or("the source was deleted")?
        } else {
            destination_quirks.calls()
        };
        let position = |op: Op, name: &str| {
            calls
                .iter()
                .position(|call| call.op == op && call.names.iter().any(|named| named == name))
        };
        for name in ["a.txt", "b.txt"] {
            let synced = position(Op::SyncData, name);
            assert_eq!(synced.is_some(), each_file, "{row}: {name}");
            if let Some(synced) = synced {
                assert!(
                    position(Op::Close, name).is_some_and(|closed| synced < closed),
                    "{row}: {name} is synced before it is closed"
                );
            }
        }
        let reached: Vec<Op> = calls
            .iter()
            .map(|call| call.op)
            .filter(|op| matches!(op, Op::SyncFs | Op::SyncDir))
            .collect();
        assert_eq!(reached, barrier, "{row}");
    }
    Ok(())
}

#[test]
fn owed_deletions_close_in_batches() -> Result<(), Box<dyn Error>> {
    use super::super::owed::Limits;
    let _serial = serial()?;
    let names = ["a", "b", "c", "d", "e"];
    let rows: [(&str, Limits, [usize; 5], Vec<Seen>); 3] = [
        (
            "the end of the paste",
            NO_LIMITS,
            [1; 5],
            vec![Seen {
                deleted: Vec::new(),
                copied: strings(&names),
                flushing: true,
                counted: 5,
            }],
        ),
        (
            "items",
            Limits {
                items: 2,
                ..NO_LIMITS
            },
            [1; 5],
            vec![
                Seen {
                    deleted: Vec::new(),
                    copied: strings(&["a", "b"]),
                    flushing: false,
                    counted: 2,
                },
                Seen {
                    deleted: strings(&["a", "b"]),
                    copied: strings(&["a", "b", "c", "d"]),
                    flushing: false,
                    counted: 4,
                },
                Seen {
                    deleted: strings(&["a", "b", "c", "d"]),
                    copied: strings(&names),
                    flushing: true,
                    counted: 5,
                },
            ],
        ),
        (
            "bytes",
            Limits {
                bytes: 10,
                ..NO_LIMITS
            },
            [10, 1, 1, 8, 12],
            vec![
                Seen {
                    deleted: Vec::new(),
                    copied: strings(&["a"]),
                    flushing: false,
                    counted: 1,
                },
                Seen {
                    deleted: strings(&["a"]),
                    copied: strings(&["a", "b", "c", "d"]),
                    flushing: false,
                    counted: 4,
                },
                Seen {
                    deleted: strings(&["a", "b", "c", "d"]),
                    copied: strings(&names),
                    flushing: true,
                    counted: 5,
                },
            ],
        ),
    ];
    for (id, (row, limits, sizes, expected)) in (310..).zip(rows) {
        let _limits = LimitsGuard::set(limits);
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let mut paths = Vec::new();
        for (name, size) in names.iter().zip(sizes) {
            fs::write(sources.join(name), vec![b'x'; size])?;
            paths.push(sources.join(name));
        }
        let barriers = Barriers::new();
        let _sources = FsQuirks::install(&sources, Quirks::default());
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                before: barriers.racers(&sources, &names, &destination, None),
                ..Quirks::ext4()
            },
        );

        let (_operation, events, early) = paste_moves(id, &destination, &paths, &barriers, None);
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { locations, .. })
                if locations.len() == names.len()
            ),
            "{row}: {:?}",
            events.borrow()
        );
        assert_eq!(barriers.seen(), expected, "{row}");
        assert!(paths.iter().all(|path| !path.exists()), "{row}");
        assert_eq!(moved_count(&events.borrow()), names.len(), "{row}");
        assert!(early.borrow().is_empty(), "{row}: {:?}", early.borrow());
    }
    Ok(())
}

#[test]
fn a_failed_barrier_deletes_nothing_in_its_batch_and_stops_the_move() -> Result<(), Box<dyn Error>>
{
    use super::super::owed::Limits;
    let _serial = serial()?;
    let _limits = LimitsGuard::set(Limits {
        items: 2,
        ..NO_LIMITS
    });
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir(&sources)?;
    fs::create_dir(&destination)?;
    let names = ["a", "b", "c", "d", "e"];
    let mut paths = Vec::new();
    for name in names {
        fs::write(sources.join(name), name)?;
        paths.push(sources.join(name));
    }
    let barriers = Barriers::new();
    let _sources = FsQuirks::install(&sources, Quirks::default());
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            before: barriers.racers(&sources, &names, &destination, None),
            fail: vec![(Op::SyncFs, "*", 1, Errno::IO)],
            ..Quirks::ext4()
        },
    );

    let (_operation, events, early) = paste_moves(320, &destination, &paths, &barriers, None);
    pump_until_transfer(&events);

    assert!(
        matches!(terminal_transfer(&events.borrow()),
            Some(OperationEvent::TransferFailed { completed_locations, message, .. })
            if completed_locations == &[Location::local(&paths[0]), Location::local(&paths[1])]
                && message.matches("writes may still be pending").count() == 1),
        "{:?}",
        events.borrow()
    );
    assert_eq!(barriers.seen().len(), 2);
    for (name, path) in names.iter().zip(&paths) {
        let moved = ["a", "b"].contains(name);
        assert_eq!(path.exists(), !moved, "{name}");
        assert_eq!(destination.join(name).exists(), *name != "e", "{name}");
    }
    assert_eq!(moved_count(&events.borrow()), 2);
    assert!(early.borrow().is_empty());
    Ok(())
}

#[test]
fn cancelling_a_move_closes_its_pending_batch() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let _limits = LimitsGuard::set(NO_LIMITS);
    // Ten items take the parallel path.
    for (id, count, cancel_after) in [(330, 3, 2), (331, 3, 3), (332, 10, 10)] {
        let row = format!("{cancel_after} of {count}");
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let names: Vec<String> = (0..count).map(|index| format!("f{index}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let paths: Vec<PathBuf> = names.iter().map(|name| sources.join(name)).collect();
        for (name, path) in names.iter().zip(&paths) {
            fs::write(path, name)?;
        }
        let barriers = Barriers::new();
        let _sources = FsQuirks::install(&sources, Quirks::default());
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                before: barriers.racers(&sources, &names, &destination, None),
                ..Quirks::ext4()
            },
        );

        let (_operation, events, early) =
            paste_moves(id, &destination, &paths, &barriers, Some(cancel_after));
        pump_until_transfer(&events);

        let (moved, left) = paths.split_at(cancel_after);
        let locations = |paths: &[PathBuf]| paths.iter().map(Location::local).collect::<Vec<_>>();
        assert!(
            matches!(terminal_transfer(&events.borrow()),
                Some(OperationEvent::Cancelled { result, .. })
                if result.completed == locations(moved)
                    && result.failed.is_empty()
                    && result.not_attempted == locations(left)),
            "{row}: {:?}",
            events.borrow()
        );
        assert_eq!(
            barriers.seen(),
            [Seen {
                deleted: Vec::new(),
                copied: strings(&names[..cancel_after]),
                flushing: true,
                counted: cancel_after,
            }],
            "{row}"
        );
        for (name, path) in names.iter().zip(&paths).take(cancel_after) {
            assert!(!path.exists(), "{row}: {name}");
            assert_eq!(fs::read_to_string(destination.join(name))?, *name, "{row}");
        }
        for (name, path) in names.iter().zip(&paths).skip(cancel_after) {
            assert!(
                path.exists() && !destination.join(name).exists(),
                "{row}: {name}"
            );
        }
        assert_eq!(moved_count(&events.borrow()), cancel_after, "{row}");
        assert!(early.borrow().is_empty(), "{row}");
    }
    Ok(())
}

#[test]
fn many_moved_files_share_a_barrier_and_are_copied_in_parallel_unless_a_disk_spins()
-> Result<(), Box<dyn Error>> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let _serial = serial()?;
    // The last item makes the batch due, which still closes as the device
    // flush shows.
    let _limits = LimitsGuard::set(super::super::owed::Limits {
        items: 12,
        ..NO_LIMITS
    });
    for (case, source_spins, destination_spins) in [
        ("no disk spins", false, false),
        ("the destination spins", false, true),
        ("a source spins", true, false),
    ] {
        let one_at_a_time = source_spins || destination_spins;
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let names: Vec<String> = (0..12).map(|index| format!("f{index:02}.txt")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut paths = Vec::new();
        for name in &names {
            fs::write(sources.join(name), name)?;
            paths.push(sources.join(name));
        }
        // The first copy waits for a second to start, as only parallel copies can.
        let (started, parallel) = (
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        let wait = if one_at_a_time {
            Duration::from_millis(300)
        } else {
            Duration::from_secs(3)
        };
        let mut opened: Vec<(Op, &'static str, usize, super::fs_quirks::Racer)> = Vec::new();
        let (count, seen_parallel) = (started.clone(), parallel.clone());
        opened.push((
            Op::OpenSource,
            "*",
            0,
            Box::new(move || {
                count.fetch_add(1, Ordering::SeqCst);
                let start = Instant::now();
                while count.load(Ordering::SeqCst) < 2 && start.elapsed() < wait {
                    thread::sleep(Duration::from_millis(5));
                }
                seen_parallel.store(count.load(Ordering::SeqCst) >= 2, Ordering::SeqCst);
            }),
        ));
        for _ in 1..names.len() {
            let count = started.clone();
            opened.push((
                Op::OpenSource,
                "*",
                0,
                Box::new(move || {
                    count.fetch_add(1, Ordering::SeqCst);
                }),
            ));
        }
        let barriers = Barriers::new();
        let changed = sources.join("f03.txt");
        let rewrite: Box<dyn FnMut() + Send> =
            Box::new(move || fs::write(&changed, b"rewritten after its copy").expect("rewrite"));
        let _sources = FsQuirks::install(
            &sources,
            Quirks {
                before: opened,
                rotational: Some(source_spins),
                ..Quirks::default()
            },
        );
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                before: barriers.racers(&sources, &names, &destination, Some(rewrite)),
                rotational: Some(destination_spins),
                ..Quirks::ext4()
            },
        );

        let (_operation, events, early) = paste_moves(340, &destination, &paths, &barriers, None);
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{case}: {:?}",
            events.borrow()
        );
        let cores = thread::available_parallelism().map_or(1, usize::from);
        assert_eq!(
            parallel.load(Ordering::SeqCst),
            !one_at_a_time && cores > 1,
            "{case}: whether the copies overlap"
        );
        assert_eq!(
            barriers.seen(),
            [Seen {
                deleted: Vec::new(),
                copied: strings(&names),
                flushing: true,
                counted: 12,
            }],
            "{case}"
        );
        for (name, path) in names.iter().zip(&paths) {
            assert_eq!(path.exists(), *name == "f03.txt", "{case}: {name}");
            assert_eq!(
                fs::read(destination.join(name))?,
                name.as_bytes(),
                "{case}: {name}"
            );
        }
        let kept: Vec<_> = events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                OperationEvent::ItemsReported { report, .. } => Some(report.kept.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(
            kept,
            [crate::services::KeptItem {
                location: Location::local(sources.join("f03.txt")),
                cause: crate::services::KeptCause::Changed,
            }],
            "{case}"
        );
        assert!(
            early
                .borrow()
                .iter()
                .all(|from| from == &Location::local(sources.join("f03.txt"))),
            "{case}"
        );
        let last = events.borrow().iter().rev().find_map(|event| match event {
            OperationEvent::TransferProgress {
                completed_files,
                total_files,
                transferred_bytes,
                total_bytes,
                ..
            } => Some((
                *completed_files,
                *total_files,
                *transferred_bytes,
                *total_bytes,
            )),
            _ => None,
        });
        let bytes: u64 = names.iter().map(|name| name.len() as u64).sum();
        assert_eq!(last, Some((12, Some(12), bytes, Some(bytes))), "{case}");
    }
    Ok(())
}

#[test]
fn names_of_one_file_moved_as_separate_items_are_all_deleted() -> Result<(), Box<dyn Error>> {
    use super::super::owed::Limits;
    let _serial = serial()?;
    let items: [(&str, &[&str]); 2] = [
        ("selected names", &["a", "b", "c"]),
        ("folders", &["daily.0", "daily.1", "daily.2"]),
    ];
    let batches = [
        NO_LIMITS,
        Limits {
            items: 1,
            ..NO_LIMITS
        },
    ];
    for (id, (&(row, names), limits)) in (350..).zip(
        items
            .iter()
            .flat_map(|item| batches.iter().map(move |limits| (item, limits))),
    ) {
        let row = format!("{row}, a batch of {}", limits.items);
        let _limits = LimitsGuard::set(*limits);
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        if row.starts_with("selected") {
            fs::write(sources.join("a"), b"shared")?;
            for name in &names[1..] {
                fs::hard_link(sources.join("a"), sources.join(name))?;
            }
        } else {
            for name in names {
                fs::create_dir(sources.join(name))?;
            }
            for file in ["f1", "f2", "f3"] {
                let first = sources.join(names[0]).join(file);
                fs::write(&first, file)?;
                for name in &names[1..] {
                    fs::hard_link(&first, sources.join(name).join(file))?;
                }
            }
        }
        let paths: Vec<PathBuf> = names.iter().map(|name| sources.join(name)).collect();
        let _volumes = separate_volumes(&sources, &destination);

        let (_operation, events, _) = paste_moves(id, &destination, &paths, &Barriers::new(), None);
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{row}: {:?}",
            events.borrow()
        );
        let kept: Vec<_> = events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                OperationEvent::ItemsReported { report, .. } => Some(report.kept.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(kept, [], "{row}");
        assert_eq!(fs::read_dir(&sources)?.count(), 0, "{row}");
        assert_eq!(moved_count(&events.borrow()), names.len(), "{row}");
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
    let at_first_unlink = Arc::new(std::sync::Mutex::new(None));
    let (record, read) = (at_first_unlink.clone(), nested.reader());
    let _sources = FsQuirks::install(
        &sources,
        Quirks {
            before: vec![(
                Op::Unlink,
                "*",
                0,
                Box::new(move || {
                    *record.lock().unwrap_or_else(|error| error.into_inner()) = Some(read());
                }),
            )],
            ..Quirks::default()
        },
    );

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(360),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(sources.join("media")),
                conflict: TransferConflict::Merge,
                target_name: None,
            }],
            move_sources: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    pump_until_transfer(&events);

    assert!(
        matches!(
            terminal_transfer(&events.borrow()),
            Some(OperationEvent::Pasted { .. })
        ),
        "{:?}",
        events.borrow()
    );
    assert_eq!(fs::read(destination.join("media/sub/new.bin"))?, b"new");
    assert!(!sources.join("media").exists());
    let calls: Vec<Op> = at_first_unlink
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
        .ok_or("a source was deleted")?
        .iter()
        .map(|call| call.op)
        .collect();
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

    glib::MainContext::default().block_on(super::super::copy_engine::sync_files(
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

#[test]
fn a_replace_on_smb_syncs_its_copy_only_where_the_original_is_deleted() -> Result<(), Box<dyn Error>>
{
    use super::fs_quirks::Trash;
    let _serial = serial()?;
    for (id, trash, synced) in [(380, Trash::Supported, false), (381, Trash::Refused, true)] {
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

        let events = run_paste(PasteRequest {
            id: OperationRequestId(id),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(sources.join("folder")),
                conflict: TransferConflict::ReplaceExisting,
                target_name: None,
            }],
            move_sources: false,
        });

        assert!(
            matches!(
                terminal_transfer(&events),
                Some(OperationEvent::Pasted { .. })
            ),
            "{row}: {events:?}"
        );
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
    }
    Ok(())
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

        let (_operation, events, _) = paste_moves(
            id,
            &destination,
            std::slice::from_ref(&source),
            &Barriers::new(),
            None,
        );
        pump_until_transfer(&events);

        if moved {
            assert!(
                matches!(
                    terminal_transfer(&events.borrow()),
                    Some(OperationEvent::Pasted { .. })
                ),
                "{row}: {:?}",
                events.borrow()
            );
            assert!(!source.exists(), "{row}");
            let calls = scope.calls();
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
        } else {
            assert!(
                matches!(terminal_transfer(&events.borrow()),
                    Some(OperationEvent::TransferFailed { message, .. })
                    if message.contains("Could not safely open")),
                "{row}: {:?}",
                events.borrow()
            );
            assert!(source.exists(), "{row}");
            assert!(!destination.join("a.txt").exists(), "{row}");
        }
    }
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
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir_all(&sources)?;
        fs::create_dir_all(&destination)?;
        let item = if conflict == TransferConflict::Merge {
            fs::create_dir(sources.join("folder"))?;
            fs::write(sources.join("folder/incoming.txt"), b"incoming")?;
            fs::create_dir(destination.join("folder"))?;
            sources.join("folder")
        } else {
            fs::write(sources.join("incoming.txt"), b"incoming")?;
            if conflict == TransferConflict::ReplaceExisting {
                fs::write(destination.join("incoming.txt"), b"old")?;
            }
            sources.join("incoming.txt")
        };
        let destination_quirks = FsQuirks::install(
            &destination,
            Quirks {
                mount_type: Some(("cifs", "rw")),
                trash: super::fs_quirks::Trash::Unsupported,
                ..Quirks::default()
            },
        );
        let at_first_unlink = Arc::new(std::sync::Mutex::new(None));
        let (record, read) = (at_first_unlink.clone(), destination_quirks.reader());
        let _sources = FsQuirks::install(
            &sources,
            Quirks {
                before: vec![(
                    Op::Unlink,
                    "*",
                    0,
                    Box::new(move || {
                        *record.lock().unwrap_or_else(|error| error.into_inner()) = Some(read());
                    }),
                )],
                ..Quirks::default()
            },
        );

        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let _operation = LocalOperationProvider.paste(
            PasteRequest {
                id: OperationRequestId(id),
                destination: Location::local(&destination),
                items: vec![PasteItem {
                    source: Location::local(&item),
                    conflict,
                    target_name: None,
                }],
                move_sources: true,
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{conflict:?}: {:?}",
            events.borrow()
        );
        assert!(!item.exists(), "{conflict:?}");
        let calls = at_first_unlink
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
            .ok_or("a source was deleted")?;
        assert!(
            calls.iter().any(|call| call.op == Op::SyncData),
            "{conflict:?}: the copy is synced before its source is deleted: {:?}",
            calls.iter().map(|call| call.op).collect::<Vec<_>>()
        );
    }
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
    for (id, ((published, tmpfile), names)) in (440..).zip(cases) {
        let context = format!("{names:?}, published by {published:?}");
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let mut paths = Vec::new();
        for name in names {
            fs::write(sources.join(name), name)?;
            paths.push(sources.join(name));
        }
        let _sources = FsQuirks::install(&sources, Quirks::default());
        type Lose = Box<dyn Fn() + Send + Sync>;
        let loser: Arc<std::sync::Mutex<Option<Lose>>> = Arc::default();
        let lose = loser.clone();
        let scope = FsQuirks::install(
            &destination,
            Quirks {
                tmpfile,
                after: vec![(
                    published,
                    "*",
                    0,
                    Box::new(move || {
                        if let Some(lose) = &*lose.lock().unwrap_or_else(|error| error.into_inner())
                        {
                            lose();
                        }
                    }),
                )],
                ..Quirks::ext4()
            },
        );
        *loser.lock().unwrap_or_else(|error| error.into_inner()) =
            Some(Box::new(scope.writeback_loser()));

        let (_operation, events, early) =
            paste_moves(id, &destination, &paths, &Barriers::new(), None);
        pump_until_transfer(&events);

        assert!(
            matches!(terminal_transfer(&events.borrow()),
                Some(OperationEvent::TransferFailed { message, .. })
                if message.contains("writes may still be pending")),
            "{context}: {:?}",
            events.borrow()
        );
        assert!(paths.iter().all(|path| path.exists()), "{context}");
        assert_eq!(moved_count(&events.borrow()), 0, "{context}");
        assert!(early.borrow().is_empty(), "{context}");
    }
    Ok(())
}

struct ClockGuard;

impl Drop for ClockGuard {
    fn drop(&mut self) {
        super::super::owed::advance_batch_clock_for_test(None);
    }
}

#[test]
fn a_batch_closes_once_its_first_entry_is_five_seconds_old() -> Result<(), Box<dyn Error>> {
    use super::super::owed::{Limits, advance_batch_clock_for_test};
    let _serial = serial()?;
    let _limits = LimitsGuard::set(Limits {
        age: Duration::from_secs(5),
        ..NO_LIMITS
    });
    let _clock = ClockGuard;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir(&sources)?;
    fs::create_dir(&destination)?;
    let names = ["a", "b", "c", "d", "e"];
    let mut paths = Vec::new();
    for name in names {
        fs::write(sources.join(name), name)?;
        paths.push(sources.join(name));
    }
    let later = |name: &'static str| -> (Op, &'static str, usize, super::fs_quirks::Racer) {
        (
            Op::OpenSource,
            name,
            0,
            Box::new(|| advance_batch_clock_for_test(Some(Duration::from_secs(3)))),
        )
    };
    let barriers = Barriers::new();
    let _sources = FsQuirks::install(
        &sources,
        Quirks {
            before: vec![later("b"), later("d")],
            ..Quirks::default()
        },
    );
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            before: barriers.racers(&sources, &names, &destination, None),
            ..Quirks::ext4()
        },
    );

    let (_operation, events, early) = paste_moves(450, &destination, &paths, &barriers, None);
    pump_until_transfer(&events);

    assert!(matches!(
        terminal_transfer(&events.borrow()),
        Some(OperationEvent::Pasted { .. })
    ));
    assert_eq!(
        barriers.seen(),
        [
            Seen {
                deleted: Vec::new(),
                copied: strings(&["a", "b", "c", "d"]),
                flushing: false,
                counted: 4,
            },
            Seen {
                deleted: strings(&["a", "b", "c", "d"]),
                copied: strings(&names),
                flushing: true,
                counted: 5,
            },
        ]
    );
    assert!(early.borrow().is_empty());
    Ok(())
}

#[test]
fn a_failed_barrier_stops_a_bulk_move() -> Result<(), Box<dyn Error>> {
    use super::super::owed::Limits;
    let _serial = serial()?;
    // Enough files for the bulk path, whose first barrier fails: the last,
    // or the first of several.
    for (id, items) in (460..).zip([usize::MAX, 3]) {
        let _limits = LimitsGuard::set(Limits { items, ..NO_LIMITS });
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let names: Vec<String> = (0..12).map(|index| format!("f{index:02}.txt")).collect();
        let mut paths = Vec::new();
        for name in &names {
            fs::write(sources.join(name), name)?;
            paths.push(sources.join(name));
        }
        let _sources = FsQuirks::install(&sources, Quirks::default());
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                fail: vec![(Op::SyncFs, "*", 0, Errno::IO)],
                ..Quirks::ext4()
            },
        );

        let (_operation, events, early) =
            paste_moves(id, &destination, &paths, &Barriers::new(), None);
        pump_until_transfer(&events);

        assert!(
            matches!(terminal_transfer(&events.borrow()),
                Some(OperationEvent::TransferFailed { message, .. })
                if message.contains("writes may still be pending")),
            "{items}: {:?}",
            events.borrow()
        );
        assert!(early.borrow().is_empty(), "{items}");
        let copied = names
            .iter()
            .filter(|name| destination.join(name).exists())
            .count();
        if items == usize::MAX {
            assert!(paths.iter().all(|path| path.exists()));
            assert_eq!(copied, names.len());
            assert_eq!(moved_count(&events.borrow()), 0);
        } else {
            assert!(copied < names.len(), "{copied} copied after the failure");
        }
    }
    Ok(())
}

#[test]
fn an_original_that_cannot_be_deleted_stays_aside_and_its_replacing_move_completes()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir(&sources)?;
    fs::create_dir(&destination)?;
    let source = sources.join("notes.txt");
    fs::write(&source, b"incoming")?;
    fs::write(destination.join("notes.txt"), b"old")?;
    let (watched, foreign) = (destination.clone(), root.path().join("foreign"));
    let racer: super::fs_quirks::Racer = Box::new(move || {
        let originals = fs::read_dir(&watched)
            .expect("list the destination")
            .filter_map(Result::ok)
            .find(|entry| entry.file_name().to_string_lossy().starts_with(".strata-"))
            .expect("the original is aside");
        fs::write(&foreign, b"foreign").expect("write the foreign file");
        fs::rename(&foreign, originals.path().join("notes.txt"))
            .expect("put it at the original's name");
    });
    let _sources = FsQuirks::install(&sources, Quirks::default());
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            trash: super::fs_quirks::Trash::Unsupported,
            before: vec![(Op::SyncFs, "*", 0, racer)],
            ..Quirks::ext4()
        },
    );
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(470),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::ReplaceExisting,
                target_name: None,
            }],
            move_sources: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    pump_until_transfer(&events);

    let events = events.take();
    assert!(
        matches!(
            terminal_transfer(&events),
            Some(OperationEvent::Pasted { .. })
        ),
        "{events:?}"
    );
    assert_eq!(fs::read(destination.join("notes.txt"))?, b"incoming");
    assert!(!source.exists());
    let [originals, _] = names(&destination)?
        .try_into()
        .map_err(|_| "the originals folder and the replacement")?;
    let kept = destination.join(originals).join("notes.txt");
    assert_eq!(fs::read(&kept)?, b"foreign");
    let reported: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::ItemsReported { report, .. } => Some(report.kept.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        matches!(
            reported.as_slice(),
            [crate::services::KeptItem {
                location,
                cause: crate::services::KeptCause::OriginalNotRemoved(_),
            }] if location == &Location::local(&kept)
        ),
        "{reported:?}"
    );
    Ok(())
}
