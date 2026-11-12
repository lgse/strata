// SPDX-License-Identifier: MIT

use std::{
    io::Write,
    os::unix::fs::{OpenOptionsExt, symlink},
};

use super::*;
use crate::test_support::temp_base::{UNNAMED_FILES, base_offers};

fn live_watch(location: Location, notify: Rc<dyn Fn(DirectoryChange)>) -> LoadHandle {
    live_watch_of(location, None, notify)
}

fn live_watch_of(
    location: Location,
    began_on: Option<ItemIdentity>,
    notify: Rc<dyn Fn(DirectoryChange)>,
) -> LoadHandle {
    let reported = Rc::new(Cell::new(None));
    let report = reported.clone();
    let handle = LocalFileSource.start_watch(
        crate::services::WatchRequest {
            location,
            include_hidden: false,
            began_on,
        },
        notify,
        Rc::new(move |watching| report.set(Some(watching))),
    );
    drain_until(|| reported.get().is_some());
    assert_eq!(reported.get(), Some(Watching::Live));
    handle
}

/// Opens a file in `directory` that has no name until it is linked in.
fn open_unnamed(directory: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_TMPFILE)
        .open(directory)
}

fn wait_for_settling(after: Duration) {
    let settled = Instant::now() + after;
    drain_until(|| Instant::now() >= settled);
}

/// Iterates the default main context until `done` or the deadline, sleeping between rounds.
fn iterate_until(deadline: Duration, mut done: impl FnMut() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + deadline;
    while !done() && Instant::now() < deadline {
        while context.iteration(false) {}
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct ReportsDrop(std::sync::mpsc::Sender<std::thread::ThreadId>);

impl Drop for ReportsDrop {
    fn drop(&mut self) {
        let _ = self.0.send(std::thread::current().id());
    }
}

type Threads = std::sync::Arc<std::sync::Mutex<Vec<std::thread::ThreadId>>>;

fn thread_recorder() -> (Threads, Box<dyn FnMut() + Send>) {
    let threads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = threads.clone();
    let record = Box::new(move || {
        recorded
            .lock()
            .expect("thread record")
            .push(std::thread::current().id());
    });
    (threads, record)
}

#[test]
fn exchanged_files_stay_listed_unless_they_are_deleted() -> Result<(), Box<dyn Error>> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};

    let _serial = serial();
    for deleted in [true, false] {
        let directory = tempfile::tempdir()?;
        let original = directory.path().join("note.txt");
        let hidden = directory.path().join(".strata-originals-test");
        fs::create_dir(&hidden)?;
        let staged = hidden.join("note.txt");
        fs::write(&original, b"old")?;
        fs::write(&staged, b"replacement")?;
        let (changes, notify) = change_recorder();
        let handle = live_watch(Location::local(directory.path()), notify);

        renameat_with(CWD, &staged, CWD, &original, RenameFlags::EXCHANGE)?;
        assert_eq!(fs::read(&original)?, b"replacement");
        if deleted {
            fs::remove_file(&original)?;
        }
        drain_until(|| !changes.borrow().is_empty());
        drop(handle);

        let location = Location::local(&original);
        let observed = changes.borrow();
        if deleted {
            assert!(
                matches!(observed.as_slice(), [DirectoryChange::Remove(path)] if path == &location),
                "a genuine deletion must remove the row: {observed:?}"
            );
        } else {
            assert!(
                matches!(observed.as_slice(), [DirectoryChange::Upsert(entry)]
                    if entry.location == location && entry.size == MetadataValue::Known(11)),
                "the replacement must stay listed with its new metadata: {observed:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_watched_folder_reports_its_removal_when_an_ancestor_was_renamed() -> Result<(), Box<dyn Error>>
{
    let _serial = serial();
    // With the identity the folder loaded with, a folder made again at the old path is
    // told apart from the one the watch follows.
    for remade in [false, true] {
        let root = tempfile::tempdir()?;
        let (ancestor, renamed) = (root.path().join("b"), root.path().join("b2"));
        fs::create_dir_all(ancestor.join("c"))?;
        let watched = Location::local(ancestor.join("c"));
        let began_on = folder_identity(&ancestor.join("c"))?;
        let (changes, notify) = change_recorder();
        let handle = live_watch_of(watched.clone(), Some(began_on), notify);

        fs::rename(&ancestor, &renamed)?;
        if remade {
            fs::create_dir_all(ancestor.join("c"))?;
        }
        fs::write(renamed.join("c/two.txt"), b"2")?;
        // The new file is looked up under the old path, where it is not found.
        let stale = Location::local(ancestor.join("c/two.txt"));
        let removed = |location: &Location| {
            changes.borrow().iter().any(
                |change| matches!(change, DirectoryChange::Remove(removed) if removed == location),
            )
        };
        drain_until(|| removed(&watched) && removed(&stale));
        drop(handle);
    }
    Ok(())
}

#[test]
fn a_look_at_the_path_that_outlasts_later_batches_still_reports_the_departure()
-> Result<(), Box<dyn Error>> {
    use crate::adapters::{FsOp, FsQuirks, Quirks};

    let _serial = serial();
    let root = tempfile::tempdir()?;
    let (ancestor, renamed) = (root.path().join("b"), root.path().join("b2"));
    fs::create_dir_all(ancestor.join("c"))?;
    let watched = Location::local(ancestor.join("c"));
    let began_on = folder_identity(&ancestor.join("c"))?;
    let gate = Gate::default();
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            after: vec![(FsOp::FolderStamp, "c", 0, gate.racer())],
            ..Quirks::default()
        },
    );
    let (changes, notify) = change_recorder();
    let handle = live_watch_of(watched.clone(), Some(began_on), notify);
    let removed = |location: &Location| {
        changes
            .borrow()
            .iter()
            .any(|change| matches!(change, DirectoryChange::Remove(removed) if removed == location))
    };

    fs::write(ancestor.join("c/one.txt"), b"1")?;
    // The first look has found the folder at its path, and waits to answer.
    drain_until(|| gate.reached());
    fs::rename(&ancestor, &renamed)?;
    // Each write is a batch of its own, flushed while the first look still waits, so only
    // the look they ask for once it ends can see the folder gone.
    for name in ["two.txt", "three.txt"] {
        fs::write(renamed.join("c").join(name), b"")?;
        let stale = Location::local(ancestor.join("c").join(name));
        drain_until(|| removed(&stale));
    }
    gate.release();
    drain_until(|| removed(&watched));
    drop(handle);
    Ok(())
}

#[test]
fn change_events_for_unnamed_temporary_files_are_ignored() -> Result<(), Box<dyn Error>> {
    let _serial = serial();
    if !base_offers(&UNNAMED_FILES)? {
        return Ok(());
    }
    let directory = unique_fixture_root("monitor-unnamed");
    fs::create_dir_all(&directory)?;
    let listed = directory.join("#1234");
    let arrived = directory.join("#5678");
    fs::write(&listed, b"a")?;
    let (changes, notify) = change_recorder();
    let handle = live_watch(Location::local(&directory), notify);
    let write_unnamed = || -> Result<Location, Box<dyn Error>> {
        let mut file = open_unnamed(&directory)?;
        file.write_all(b"never linked")?;
        file.set_permissions(fs::Permissions::from_mode(0o644))?;
        Ok(Location::local(
            directory.join(format!("#{}", file.metadata()?.ino())),
        ))
    };
    let upserted = |path: &Path, size: u64| {
        let location = Location::local(path);
        let changes = changes.clone();
        move || {
            changes.borrow().iter().any(|change| {
                matches!(change, DirectoryChange::Upsert(entry)
                    if entry.location == location && entry.size == MetadataValue::Known(size))
            })
        }
    };

    let mut unnamed = vec![write_unnamed()?];
    fs::write(&listed, b"updated")?;
    drain_until(upserted(&listed, 7));
    unnamed.push(write_unnamed()?);
    fs::write(&arrived, b"")?;
    drain_until(upserted(&arrived, 0));
    fs::write(&arrived, b"grown")?;
    drain_until(upserted(&arrived, 5));
    wait_for_settling(Duration::from_millis(300));
    drop(handle);
    fs::remove_dir_all(&directory)?;

    let changes = changes.borrow();
    assert!(
        changes.iter().all(|change| match change {
            DirectoryChange::Upsert(entry) => !unnamed.contains(&entry.location),
            DirectoryChange::Remove(location) => !unnamed.contains(location),
            DirectoryChange::Move { .. } | DirectoryChange::Rescan => false,
        }),
        "only the named files should be queried: {changes:?}"
    );
    Ok(())
}

#[test]
fn a_file_linked_into_place_is_queried_once() -> Result<(), Box<dyn Error>> {
    use std::os::fd::AsRawFd;

    let _serial = serial();
    if !base_offers(&UNNAMED_FILES)? {
        return Ok(());
    }
    let directory = unique_fixture_root("monitor-linked");
    fs::create_dir_all(&directory)?;
    let (changes, notify) = change_recorder();
    let handle = live_watch(Location::local(&directory), notify);
    let linked = Location::local(directory.join("linked.txt"));
    let queries = || {
        changes
            .borrow()
            .iter()
            .filter(|change| matches!(change, DirectoryChange::Upsert(entry) if entry.location == linked))
            .count()
    };

    let mut file = open_unnamed(&directory)?;
    file.write_all(b"contents")?;
    rustix::fs::linkat(
        rustix::fs::CWD,
        format!("/proc/self/fd/{}", file.as_raw_fd()),
        rustix::fs::CWD,
        directory.join("linked.txt"),
        rustix::fs::AtFlags::SYMLINK_FOLLOW,
    )?;
    drop(file);
    drain_until(|| queries() > 0);
    // GLib sends the linked name's changes-done hint two seconds later.
    wait_for_settling(Duration::from_millis(2_500));
    drop(handle);
    fs::remove_dir_all(&directory)?;

    assert_eq!(queries(), 1, "{:?}", changes.borrow());
    Ok(())
}

#[test]
fn a_dangling_link_arriving_in_a_watched_folder_is_reported_as_a_link() -> Result<(), Box<dyn Error>>
{
    let _serial = serial();
    let directory = unique_fixture_root("monitor-dangling-link");
    fs::create_dir_all(&directory)?;
    let (changes, notify) = change_recorder();
    let handle = live_watch(Location::local(&directory), notify);
    let link = Location::local(directory.join("broken-link.txt"));

    symlink(
        directory.join("missing.txt"),
        directory.join("broken-link.txt"),
    )?;
    drain_until(|| {
        changes.borrow().iter().any(|change| {
            matches!(change, DirectoryChange::Upsert(entry)
                if entry.location == link && entry.kind == EntryKind::SymbolicLink)
        })
    });
    drop(handle);
    fs::remove_dir_all(&directory)?;
    Ok(())
}

#[test]
fn sustained_native_monitor_bursts_flush_without_rescanning() {
    let _serial = serial();
    let directory = FixtureDir::new("monitor-burst");
    let (changes, notify) = change_recorder();
    let handle = live_watch(directory.location(), notify);

    for index in 0..1_000 {
        directory.write(&format!("file-{index:04}.txt"), b"real contents");
    }

    let upserts = || {
        let observed = changes.borrow();
        assert!(
            observed
                .iter()
                .all(|change| !matches!(change, DirectoryChange::Rescan)),
            "a bounded native burst should stay incremental"
        );
        observed
            .iter()
            .filter(|change| matches!(change, DirectoryChange::Upsert(_)))
            .count()
    };
    iterate_until(Duration::from_secs(10), || upserts() == 1_000);
    assert_eq!(upserts(), 1_000, "all monitored entries should arrive");
    drop(handle);
    directory.remove();
}

#[test]
fn watching_a_uri_location_reports_created_entries() {
    let _serial = serial();
    let directory = FixtureDir::new("uri-watch");
    let uri =
        glib::filename_to_uri(directory.path(), None).expect("the fixture path should have a URI");
    let (changes, notify) = change_recorder();
    let handle = live_watch(Location::uri(uri.as_str()), notify);

    directory.write("arrival.txt", b"arrived");

    iterate_until(Duration::from_secs(10), || !changes.borrow().is_empty());
    drop(handle);
    let observed = changes.borrow().clone();
    directory.remove();

    assert!(
        observed.iter().any(|change| matches!(
            change,
            DirectoryChange::Upsert(entry) if entry.native_name == "arrival.txt"
        )),
        "the monitor should report the new entry: {observed:?}"
    );
}

#[test]
fn a_monitor_is_made_and_released_on_a_worker_while_the_window_keeps_running()
-> Result<(), Box<dyn Error>> {
    use crate::adapters::{FsOp, FsQuirks, Quirks};

    let _serial = serial();
    for dropped_while_parked in [false, true] {
        let root = tempfile::tempdir()?;
        let folder = root.path().join("folder");
        fs::create_dir(&folder)?;
        let gate = Gate::default();
        let (released_on, released) = thread_recorder();
        let scope = FsQuirks::install(
            root.path(),
            Quirks {
                before: vec![(FsOp::Watch, "folder", 0, gate.racer())],
                after: vec![(FsOp::Unwatch, "folder", 0, released)],
                ..Quirks::default()
            },
        );
        let (dropped, drops) = std::sync::mpsc::channel();
        let probe = ReportsDrop(dropped);
        let (changes, collected) = change_recorder();
        let reported = Rc::new(Cell::new(None));
        let report = reported.clone();
        let ticked = Rc::new(Cell::new(false));
        let tick = ticked.clone();
        glib::timeout_add_local_once(Duration::from_millis(10), move || tick.set(true));

        let handle = LocalFileSource.start_watch(
            crate::services::WatchRequest {
                location: Location::local(&folder),
                include_hidden: false,
                began_on: None,
            },
            Rc::new(move |change| {
                let _held = &probe;
                collected(change);
            }),
            Rc::new(move |watching| report.set(Some(watching))),
        );
        drain_until(|| gate.reached() && ticked.get());
        assert_eq!(reported.get(), None, "parked");
        if dropped_while_parked {
            drop(handle);
            gate.release();
        } else {
            gate.release();
            drain_until(|| reported.get().is_some());
            assert_eq!(reported.get(), Some(Watching::Live));
            fs::write(folder.join("arrival"), b"arrived")?;
            drain_until(|| !changes.borrow().is_empty());
            drop(handle);
            assert_eq!(
                drops.try_recv().ok(),
                Some(std::thread::current().id()),
                "the handler's state goes with the handle, on this thread"
            );
        }
        drain_until(|| scope.count(FsOp::Unwatch) == 1);

        assert_eq!(reported.get().is_none(), dropped_while_parked);
        assert!(
            !released_on
                .lock()
                .expect("thread record")
                .contains(&std::thread::current().id()),
            "dropped while parked: {dropped_while_parked}"
        );
    }
    Ok(())
}

#[test]
fn a_folder_on_a_mount_that_stopped_answering_gets_no_monitor() -> Result<(), Box<dyn Error>> {
    use crate::adapters::{FsOp, FsQuirks, Quirks, mount_lanes};

    let _serial = serial();
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    fs::create_dir(&folder)?;
    let gate = Gate::default();
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(FsOp::FolderStamp, "folder", 0, gate.racer())],
            ..Quirks::default()
        },
    );
    let watch = || {
        let reported = Rc::new(Cell::new(None));
        let report = reported.clone();
        let handle = LocalFileSource.start_watch(
            crate::services::WatchRequest {
                location: Location::local(&folder),
                include_hidden: false,
                began_on: None,
            },
            Rc::new(|_| {}),
            Rc::new(move |watching| report.set(Some(watching))),
        );
        drain_until(|| reported.get().is_some());
        (reported.get(), handle)
    };
    let events = Events::default();
    let load = load_into(&events, &folder, 1);
    drain_until(|| mount_lanes::responsiveness(&folder) == mount_lanes::Responsiveness::Stalled);

    let (stalled, _unwatched) = watch();
    gate.release();
    drain_until(|| outcome_of(&events, 1).is_some());
    let (answering, _watched) = watch();

    assert_eq!(
        (stalled, answering, scope.count(FsOp::Watch)),
        (Some(Watching::Unavailable), Some(Watching::Live), 1)
    );
    drop(load);
    Ok(())
}

#[test]
fn a_watched_path_counts_as_departed_unless_it_holds_the_same_folder() -> Result<(), Box<dyn Error>>
{
    type Change = fn(&Path) -> std::io::Result<()>;
    // Each row changes the watched folder `b/c`, read with and without the identity the
    // watch began on.
    let rows: [(&str, Change, bool, bool); 4] = [
        ("unchanged", |_| Ok(()), false, false),
        (
            "ancestor renamed",
            |root| fs::rename(root.join("b"), root.join("b2")),
            true,
            true,
        ),
        (
            "ancestor renamed and the path made again",
            |root| {
                fs::rename(root.join("b"), root.join("b2"))?;
                fs::create_dir_all(root.join("b/c"))
            },
            true,
            false,
        ),
        (
            "replaced by a file",
            |root| {
                fs::remove_dir(root.join("b/c"))?;
                fs::write(root.join("b/c"), "")
            },
            true,
            true,
        ),
    ];
    for (row, change, departed, departed_unidentified) in rows {
        let root = tempfile::tempdir()?;
        let watched = root.path().join("b/c");
        fs::create_dir_all(&watched)?;
        let began_on = folder_identity(&watched)?;
        change(root.path())?;

        let progress = Progress::unlaned();
        assert_eq!(
            (
                departed_from(&watched, Some(began_on), &progress),
                departed_from(&watched, None, &progress)
            ),
            (departed, departed_unidentified),
            "{row}"
        );
    }
    Ok(())
}
