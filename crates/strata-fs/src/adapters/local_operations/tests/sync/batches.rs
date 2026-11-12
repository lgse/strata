// SPDX-License-Identifier: MIT

use super::*;

const FIVE: [&str; 5] = ["a", "b", "c", "d", "e"];

/// How a batch is limited, the sizes of the five moved files, and what
/// their barriers see.
type BatchRow = (&'static str, Limits, [usize; 5], Vec<Seen>);

fn check_owed_batches(
    id: u64,
    (row, limits, sizes, expected): BatchRow,
) -> Result<(), Box<dyn Error>> {
    let _limits = LimitsGuard::set(limits);
    let root = tempfile::tempdir()?;
    let layout = Layout::create(root.path(), FIVE)?;
    for (path, size) in layout.paths().iter().zip(sizes) {
        fs::write(path, vec![b'x'; size])?;
    }
    let barriers = Barriers::new();
    let _sources = FsQuirks::install(&layout.sources, Quirks::default());
    let _destination = FsQuirks::install(
        &layout.destination,
        Quirks {
            before: barriers.racers(&layout, None),
            ..Quirks::ext4()
        },
    );

    let moved = layout.move_all(id, &barriers);

    assert!(
        matches!(
            terminal_transfer(&moved.events()),
            Some(OperationEvent::Pasted { locations, .. })
            if locations.len() == FIVE.len()
        ),
        "{row}: {:?}",
        moved.events()
    );
    assert_eq!(barriers.seen(), expected, "{row}");
    assert!(layout.paths().iter().all(|path| !path.exists()), "{row}");
    moved.assert_moved_after_deletion(FIVE.len(), row);
    Ok(())
}

#[test]
fn owed_deletions_close_in_batches() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let rows: [BatchRow; 3] = [
        (
            "the end of the paste",
            NO_LIMITS,
            [1; 5],
            vec![seen(&[], &FIVE, true, 5)],
        ),
        (
            "items",
            Limits {
                items: 2,
                ..NO_LIMITS
            },
            [1; 5],
            vec![
                seen(&[], &["a", "b"], false, 2),
                seen(&["a", "b"], &["a", "b", "c", "d"], false, 4),
                seen(&["a", "b", "c", "d"], &FIVE, true, 5),
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
                seen(&[], &["a"], false, 1),
                seen(&["a"], &["a", "b", "c", "d"], false, 4),
                seen(&["a", "b", "c", "d"], &FIVE, true, 5),
            ],
        ),
    ];
    for (id, row) in (310..).zip(rows) {
        check_owed_batches(id, row)?;
    }
    Ok(())
}

#[test]
fn a_failed_barrier_deletes_nothing_in_its_batch_and_stops_the_move() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let _limits = LimitsGuard::set(Limits {
        items: 2,
        ..NO_LIMITS
    });
    let root = tempfile::tempdir()?;
    let layout = Layout::create(root.path(), FIVE)?;
    layout.write_sources(|name| name.as_bytes().to_vec())?;
    let paths = layout.paths();
    let barriers = Barriers::new();
    let _sources = FsQuirks::install(&layout.sources, Quirks::default());
    let _destination = FsQuirks::install(
        &layout.destination,
        Quirks {
            before: barriers.racers(&layout, None),
            fail: vec![(Op::SyncFs, "*", 1, Errno::IO)],
            ..Quirks::ext4()
        },
    );

    let moved = layout.move_all(320, &barriers);

    assert!(
        matches!(terminal_transfer(&moved.events()),
            Some(OperationEvent::TransferFailed { completed_locations, message, .. })
            if completed_locations == &[Location::local(&paths[0]), Location::local(&paths[1])]
                && message.matches("writes may still be pending").count() == 1),
        "{:?}",
        moved.events()
    );
    assert_eq!(barriers.seen().len(), 2);
    for (name, path) in FIVE.iter().zip(&paths) {
        let gone = ["a", "b"].contains(name);
        assert_eq!(path.exists(), !gone, "{name}");
        assert_eq!(
            layout.destination.join(name).exists(),
            *name != "e",
            "{name}"
        );
    }
    moved.assert_moved_after_deletion(2, "");
    Ok(())
}

fn check_cancelled_batch(id: u64, count: usize, cancel_after: usize) -> Result<(), Box<dyn Error>> {
    let row = format!("{cancel_after} of {count}");
    let root = tempfile::tempdir()?;
    let layout = Layout::create(root.path(), (0..count).map(|index| format!("f{index}")))?;
    layout.write_sources(|name| name.as_bytes().to_vec())?;
    let (names, paths) = (layout.names(), layout.paths());
    let barriers = Barriers::new();
    let _sources = FsQuirks::install(&layout.sources, Quirks::default());
    let _destination = FsQuirks::install(
        &layout.destination,
        Quirks {
            before: barriers.racers(&layout, None),
            ..Quirks::ext4()
        },
    );

    let moved = paste_moves(
        Moving {
            id,
            destination: &layout.destination,
            sources: &paths,
            cancel_after: Some(cancel_after),
        },
        &barriers,
    );

    let (done, left) = paths.split_at(cancel_after);
    let locations = |paths: &[PathBuf]| paths.iter().map(Location::local).collect::<Vec<_>>();
    assert!(
        matches!(terminal_transfer(&moved.events()),
            Some(OperationEvent::Cancelled { result, .. })
            if result.completed == locations(done)
                && result.failed.is_empty()
                && result.not_attempted == locations(left)),
        "{row}: {:?}",
        moved.events()
    );
    assert_eq!(
        barriers.seen(),
        [seen(&[], &names[..cancel_after], true, cancel_after)],
        "{row}"
    );
    let (moved_names, left_names) = names.split_at(cancel_after);
    for (name, path) in moved_names.iter().zip(done) {
        assert!(!path.exists(), "{row}: {name}");
        let copy = fs::read_to_string(layout.destination.join(name))?;
        assert_eq!(copy, *name, "{row}");
    }
    for (name, path) in left_names.iter().zip(left) {
        assert!(
            path.exists() && !layout.destination.join(name).exists(),
            "{row}: {name}"
        );
    }
    moved.assert_moved_after_deletion(cancel_after, &row);
    Ok(())
}

#[test]
fn cancelling_a_move_closes_its_pending_batch() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let _limits = LimitsGuard::set(NO_LIMITS);
    // Ten items take the parallel path.
    for (id, count, cancel_after) in [(330, 3, 2), (331, 3, 3), (332, 10, 10)] {
        check_cancelled_batch(id, count, cancel_after)?;
    }
    Ok(())
}

/// Copies started across racers before each source opens, and whether the
/// first saw a second start while it waited.
struct Overlap {
    started: Arc<AtomicUsize>,
    parallel: Arc<AtomicBool>,
}

impl Overlap {
    fn new() -> Self {
        Self {
            started: Arc::default(),
            parallel: Arc::default(),
        }
    }

    /// The first of `count` racers waits up to `wait` for a second copy to
    /// start, as only parallel copies can.
    fn racers(&self, count: usize, wait: Duration) -> Vec<Before> {
        let (started, parallel) = (self.started.clone(), self.parallel.clone());
        let first: Racer = Box::new(move || {
            started.fetch_add(1, Ordering::SeqCst);
            let start = Instant::now();
            while started.load(Ordering::SeqCst) < 2 && start.elapsed() < wait {
                thread::sleep(Duration::from_millis(5));
            }
            parallel.store(started.load(Ordering::SeqCst) >= 2, Ordering::SeqCst);
        });
        let mut racers: Vec<Before> = vec![(Op::OpenSource, "*", 0, first)];
        for _ in 1..count {
            let started = self.started.clone();
            let next: Racer = Box::new(move || {
                started.fetch_add(1, Ordering::SeqCst);
            });
            racers.push((Op::OpenSource, "*", 0, next));
        }
        racers
    }

    fn seen_parallel(&self) -> bool {
        self.parallel.load(Ordering::SeqCst)
    }
}

/// The last progress as completed files, total files, transferred bytes,
/// and total bytes.
fn last_progress(events: &[OperationEvent]) -> Option<(usize, Option<usize>, u64, Option<u64>)> {
    events.iter().rev().find_map(|event| match event {
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
    })
}

fn check_spinning_disks(
    case: &str,
    source_spins: bool,
    destination_spins: bool,
) -> Result<(), Box<dyn Error>> {
    let one_at_a_time = source_spins || destination_spins;
    let root = tempfile::tempdir()?;
    let layout = Layout::create(root.path(), (0..12).map(|index| format!("f{index:02}.txt")))?;
    layout.write_sources(|name| name.as_bytes().to_vec())?;
    let wait = if one_at_a_time {
        Duration::from_millis(300)
    } else {
        Duration::from_secs(3)
    };
    let overlap = Overlap::new();
    let barriers = Barriers::new();
    let changed = layout.sources.join("f03.txt");
    let rewritten = changed.clone();
    let rewrite: Racer =
        Box::new(move || fs::write(&rewritten, b"rewritten after its copy").expect("rewrite"));
    let _sources = FsQuirks::install(
        &layout.sources,
        Quirks {
            before: overlap.racers(layout.names.len(), wait),
            rotational: Some(source_spins),
            ..Quirks::default()
        },
    );
    let _destination = FsQuirks::install(
        &layout.destination,
        Quirks {
            before: barriers.racers(&layout, Some(rewrite)),
            rotational: Some(destination_spins),
            ..Quirks::ext4()
        },
    );

    let moved = layout.move_all(340, &barriers);

    assert_pasted(&moved.events(), case);
    let cores = thread::available_parallelism().map_or(1, usize::from);
    assert_eq!(
        overlap.seen_parallel(),
        !one_at_a_time && cores > 1,
        "{case}: whether the copies overlap"
    );
    assert_eq!(
        barriers.seen(),
        [seen(&[], &layout.names(), true, 12)],
        "{case}"
    );
    assert_kept_only_the_changed_source(&layout, &moved, &changed, case)?;
    let bytes: u64 = layout.names.iter().map(|name| name.len() as u64).sum();
    assert_eq!(
        last_progress(&moved.events()),
        Some((12, Some(12), bytes, Some(bytes))),
        "{case}"
    );
    Ok(())
}

/// Every source but `changed` went, every copy is whole, and only `changed`
/// is reported kept, as changed, or moved while it remained.
fn assert_kept_only_the_changed_source(
    layout: &Layout,
    moved: &Moved,
    changed: &Path,
    case: &str,
) -> Result<(), Box<dyn Error>> {
    for (name, path) in layout.names().into_iter().zip(layout.paths()) {
        assert_eq!(path.exists(), path == changed, "{case}: {name}");
        let copy = fs::read(layout.destination.join(name))?;
        assert_eq!(copy, name.as_bytes(), "{case}: {name}");
    }
    assert_eq!(
        kept_items(&moved.events()),
        [KeptItem {
            location: Location::local(changed),
            cause: KeptCause::Changed,
        }],
        "{case}"
    );
    assert!(
        moved
            .early
            .borrow()
            .iter()
            .all(|from| from == &Location::local(changed)),
        "{case}"
    );
    Ok(())
}

#[test]
fn many_moved_files_share_a_barrier_and_are_copied_in_parallel_unless_a_disk_spins()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // The last item makes the batch due, which still closes as the device
    // flush shows.
    let _limits = LimitsGuard::set(Limits {
        items: 12,
        ..NO_LIMITS
    });
    for (case, source_spins, destination_spins) in [
        ("no disk spins", false, false),
        ("the destination spins", false, true),
        ("a source spins", true, false),
    ] {
        check_spinning_disks(case, source_spins, destination_spins)?;
    }
    Ok(())
}

/// Makes `names` beneath `sources` names of one file.
fn link_files(sources: &Path, names: &[&str]) -> io::Result<()> {
    fs::write(sources.join(names[0]), b"shared")?;
    for name in &names[1..] {
        fs::hard_link(sources.join(names[0]), sources.join(name))?;
    }
    Ok(())
}

/// Makes `names` beneath `sources` folders whose files are each one file
/// under every folder.
fn link_folders(sources: &Path, names: &[&str]) -> io::Result<()> {
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
    Ok(())
}

fn check_linked_names(
    id: u64,
    row: &str,
    names: &[&str],
    limits: Limits,
) -> Result<(), Box<dyn Error>> {
    let row = format!("{row}, a batch of {}", limits.items);
    let _limits = LimitsGuard::set(limits);
    let root = tempfile::tempdir()?;
    let layout = Layout::create(root.path(), names)?;
    if row.starts_with("selected") {
        link_files(&layout.sources, names)?;
    } else {
        link_folders(&layout.sources, names)?;
    }
    let _volumes = separate_volumes(&layout.sources, &layout.destination);

    let moved = layout.move_all(id, &Barriers::new());

    assert_pasted(&moved.events(), &row);
    assert_eq!(kept_items(&moved.events()), [], "{row}");
    assert_eq!(fs::read_dir(&layout.sources)?.count(), 0, "{row}");
    assert_eq!(moved.moved_count(), names.len(), "{row}");
    Ok(())
}

#[test]
fn names_of_one_file_moved_as_separate_items_are_all_deleted() -> Result<(), Box<dyn Error>> {
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
    let cases = items
        .iter()
        .flat_map(|item| batches.iter().map(move |limits| (item, limits)));
    for (id, (&(row, names), limits)) in (350..).zip(cases) {
        check_linked_names(id, row, names, *limits)?;
    }
    Ok(())
}

struct ClockGuard;

impl Drop for ClockGuard {
    fn drop(&mut self) {
        super::super::super::owed::advance_batch_clock_for_test(None);
    }
}

/// Advances the batch clock by three seconds as `name` is opened.
fn three_seconds_on_opening(name: &'static str) -> Before {
    (
        Op::OpenSource,
        name,
        0,
        Box::new(|| {
            super::super::super::owed::advance_batch_clock_for_test(Some(Duration::from_secs(3)));
        }),
    )
}

#[test]
fn a_batch_closes_once_its_first_entry_is_five_seconds_old() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let _limits = LimitsGuard::set(Limits {
        age: Duration::from_secs(5),
        ..NO_LIMITS
    });
    let _clock = ClockGuard;
    let root = tempfile::tempdir()?;
    let layout = Layout::create(root.path(), FIVE)?;
    layout.write_sources(|name| name.as_bytes().to_vec())?;
    let barriers = Barriers::new();
    let _sources = FsQuirks::install(
        &layout.sources,
        Quirks {
            before: vec![three_seconds_on_opening("b"), three_seconds_on_opening("d")],
            ..Quirks::default()
        },
    );
    let _destination = FsQuirks::install(
        &layout.destination,
        Quirks {
            before: barriers.racers(&layout, None),
            ..Quirks::ext4()
        },
    );

    let moved = layout.move_all(450, &barriers);

    assert_pasted(&moved.events(), "");
    assert_eq!(
        barriers.seen(),
        [
            seen(&[], &["a", "b", "c", "d"], false, 4),
            seen(&["a", "b", "c", "d"], &FIVE, true, 5),
        ]
    );
    assert!(moved.early.borrow().is_empty());
    Ok(())
}

fn check_failed_bulk_barrier(id: u64, items: usize) -> Result<(), Box<dyn Error>> {
    let _limits = LimitsGuard::set(Limits { items, ..NO_LIMITS });
    let root = tempfile::tempdir()?;
    let layout = Layout::create(root.path(), (0..12).map(|index| format!("f{index:02}.txt")))?;
    layout.write_sources(|name| name.as_bytes().to_vec())?;
    let _sources = FsQuirks::install(&layout.sources, Quirks::default());
    let _destination = FsQuirks::install(
        &layout.destination,
        Quirks {
            fail: vec![(Op::SyncFs, "*", 0, Errno::IO)],
            ..Quirks::ext4()
        },
    );

    let moved = layout.move_all(id, &Barriers::new());

    assert!(
        matches!(terminal_transfer(&moved.events()),
            Some(OperationEvent::TransferFailed { message, .. })
            if message.contains("writes may still be pending")),
        "{items}: {:?}",
        moved.events()
    );
    assert!(moved.early.borrow().is_empty(), "{items}");
    let copied = layout
        .names
        .iter()
        .filter(|name| layout.destination.join(name).exists())
        .count();
    if items == usize::MAX {
        assert!(layout.paths().iter().all(|path| path.exists()));
        assert_eq!(copied, layout.names.len());
        assert_eq!(moved.moved_count(), 0);
    } else {
        assert!(
            copied < layout.names.len(),
            "{copied} copied after the failure"
        );
    }
    Ok(())
}

#[test]
fn a_failed_barrier_stops_a_bulk_move() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // Enough files for the bulk path, whose first barrier fails: the last,
    // or the first of several.
    for (id, items) in (460..).zip([usize::MAX, 3]) {
        check_failed_bulk_barrier(id, items)?;
    }
    Ok(())
}

/// Before the barrier, puts a foreign file at the name of the original the
/// replacing move set aside in `destination`.
fn foreign_file_at_the_original(destination: PathBuf, foreign: PathBuf) -> Racer {
    Box::new(move || {
        let originals = fs::read_dir(&destination)
            .expect("list the destination")
            .filter_map(Result::ok)
            .find(|entry| entry.file_name().to_string_lossy().starts_with(".strata-"))
            .expect("the original is aside");
        fs::write(&foreign, b"foreign").expect("write the foreign file");
        fs::rename(&foreign, originals.path().join("notes.txt"))
            .expect("put it at the original's name");
    })
}

#[test]
fn an_original_that_cannot_be_deleted_stays_aside_and_its_replacing_move_completes()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let layout = Layout::create(root.path(), ["notes.txt"])?;
    let (sources, destination) = (&layout.sources, &layout.destination);
    let source = sources.join("notes.txt");
    fs::write(&source, b"incoming")?;
    fs::write(destination.join("notes.txt"), b"old")?;
    let racer = foreign_file_at_the_original(destination.clone(), root.path().join("foreign"));
    let _sources = FsQuirks::install(sources, Quirks::default());
    let _destination = FsQuirks::install(
        destination,
        Quirks {
            trash: Trash::Unsupported,
            before: vec![(Op::SyncFs, "*", 0, racer)],
            ..Quirks::ext4()
        },
    );

    let items = vec![item(&source, TransferConflict::ReplaceExisting)];
    let (_operation, events) = paste_pumped(request(470, destination, items, true));

    assert_pasted(&events, "");
    assert_eq!(fs::read(destination.join("notes.txt"))?, b"incoming");
    assert!(!source.exists());
    let [originals, _] = names(destination)?
        .try_into()
        .map_err(|_| "the originals folder and the replacement")?;
    let kept = destination.join(originals).join("notes.txt");
    assert_eq!(fs::read(&kept)?, b"foreign");
    let reported = kept_items(&events);
    assert!(
        matches!(
            reported.as_slice(),
            [KeptItem {
                location,
                cause: KeptCause::OriginalNotRemoved(_),
            }] if location == &Location::local(&kept)
        ),
        "{reported:?}"
    );
    Ok(())
}
