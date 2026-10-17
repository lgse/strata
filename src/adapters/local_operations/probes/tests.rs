// SPDX-License-Identifier: MIT

use std::{
    fs, io,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

use gtk::{gio, glib};
use rustix::io::Errno;

use super::{Presence, child_folders, folder, folder_was_deleted, presence};
use crate::{
    adapters::{
        local_operations::tests::fs_quirks::{FsQuirks, Op, Quirks, Racer},
        volume::{MountTable, mount_lanes, mount_watch},
    },
    model::{EntryKind, Location},
    services::{LocationValidationError, file_activity},
};

fn answer<T>(probe: impl Future<Output = T>) -> T {
    glib::MainContext::new().block_on(probe)
}

fn summary(presence: &Presence) -> String {
    match presence {
        Presence::Absent => "absent".to_owned(),
        Presence::Present(kind) => format!("{kind:?}"),
        Presence::Unknown(error) => format!("unknown {}", error.raw_os_error().unwrap_or_default()),
    }
}

#[test]
fn presence_reports_absence_only_where_it_is_proven() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("file"), b"content")?;
    fs::write(root.path().join("flaky"), b"content")?;
    fs::create_dir(root.path().join("folder"))?;
    std::os::unix::fs::symlink(root.path().join("nowhere"), root.path().join("dangling"))?;
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            fail: vec![(Op::Stat, "flaky", 0, Errno::IO)],
            ..Quirks::default()
        },
    );
    let rows = [
        ("file", "File"),
        ("folder", "Directory"),
        ("dangling", "SymbolicLink"),
        ("missing", "absent"),
        ("gone/child", "absent"),
        ("file/child", "absent"),
        ("flaky", "unknown 5"),
    ];
    let locations: Vec<_> = rows
        .iter()
        .map(|(name, _)| Location::local(root.path().join(name)))
        .collect();

    let found = answer(presence(&locations, &gio::Cancellable::new()));

    let found: Vec<String> = found.iter().map(summary).collect();
    let expected: Vec<&str> = rows.iter().map(|(_, expected)| *expected).collect();
    assert_eq!(found, expected);
    Ok(())
}

#[test]
fn presence_of_an_unreachable_remote_location_is_unknown() {
    let found = answer(presence(
        &[Location::uri("sftp://strata-probe-test.invalid/missing")],
        &gio::Cancellable::new(),
    ));
    assert!(
        matches!(found.as_slice(), [Presence::Unknown(_)]),
        "{found:?}"
    );
}

#[test]
fn a_folder_checks_out_once_it_can_be_listed() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("folder"))?;
    fs::create_dir(root.path().join("locked"))?;
    fs::write(root.path().join("file"), b"content")?;
    std::os::unix::fs::symlink(root.path().join("folder"), root.path().join("link"))?;
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            fail: vec![(Op::OpenPath, "*", 2, Errno::ACCESS)],
            ..Quirks::default()
        },
    );
    for (name, expected) in [
        ("folder", Ok(())),
        ("link", Ok(())),
        ("file", Err(LocationValidationError::NotDirectory)),
        ("missing", Err(LocationValidationError::Missing)),
        ("locked", Err(LocationValidationError::Inaccessible)),
    ] {
        assert_eq!(answer(folder(&root.path().join(name))), expected, "{name}");
    }
    Ok(())
}

#[test]
fn a_folder_counts_as_deleted_only_beside_others_and_away_from_mount_points() {
    crate::test_support::gtk_test(
        "adapters::local_operations::probes::tests::a_folder_counts_as_deleted_only_beside_others_and_away_from_mount_points",
        || {
            let root = tempfile::tempdir().expect("fixture");
            let crowded = root.path().join("crowded");
            let empty = root.path().join("empty");
            let mounts = root.path().join("mounts");
            for folder in [&crowded, &empty, &mounts] {
                fs::create_dir(folder).expect("fixture folder");
            }
            for parent in [&crowded, &mounts] {
                fs::create_dir(parent.join("kept")).expect("fixture folder");
            }
            mount_watch::publish_for_isolated_test(Some(MountTable::parse(format!(
                "1 0 8:1 / / rw - ext4 /dev/root rw\n900 1 0:90 / {} rw - vfat /dev/sdb1 rw\n",
                mounts.join("drive").display()
            ))));
            for (path, deleted) in [
                (crowded.join("gone"), true),
                (crowded.join("kept"), false),
                (empty.join("gone"), false),
                (mounts.join("gone"), false),
            ] {
                assert_eq!(
                    answer(folder_was_deleted(&path)),
                    deleted,
                    "{}",
                    path.display()
                );
            }
        },
    );
}

#[test]
fn child_folders_match_their_prefix_through_symbolic_links()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    for folder in ["Docs", "Downloads", ".dotfiles", "other"] {
        fs::create_dir(root.path().join(folder))?;
    }
    fs::write(root.path().join("dots.txt"), b"content")?;
    std::os::unix::fs::symlink(root.path().join("other"), root.path().join("documents"))?;
    std::os::unix::fs::symlink(root.path().join("dots.txt"), root.path().join("dotlink"))?;
    let listed = |prefix: &str, include_hidden: bool, scanned: usize| {
        let mut names = answer(child_folders(root.path(), prefix, include_hidden, scanned));
        names.sort();
        names
    };

    assert_eq!(listed("do", false, 100), ["Docs", "Downloads", "documents"]);
    assert_eq!(listed(".", true, 100), [".dotfiles"]);
    assert_eq!(listed("", false, 100).len(), 4);
    assert!(listed("", true, 2).len() <= 2);

    let operation = file_activity::begin(file_activity::OperationScope::default());
    let fence = operation.fence([root.path()]);
    assert!(
        listed("do", false, 100).is_empty(),
        "an operation changes the folder"
    );
    drop(fence);
    assert_eq!(listed("do", false, 100).len(), 3);
    Ok(())
}

#[derive(Clone, Default)]
struct Gate(Arc<(Mutex<(bool, bool)>, Condvar)>);

impl Gate {
    fn racer(&self) -> Racer {
        let gate = self.clone();
        Box::new(move || {
            let (state, changed) = &*gate.0;
            let mut state = state.lock().expect("gate");
            state.0 = true;
            changed.notify_all();
            while !state.1 {
                state = changed.wait(state).expect("gate");
            }
        })
    }

    fn reached(&self) -> bool {
        self.0.0.lock().expect("gate").0
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        self.0.0.lock().expect("gate").1 = true;
        self.0.1.notify_all();
    }
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn probes_of_a_folder_that_stopped_answering_time_out_at_once()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("stuck"), b"content")?;
    fs::write(root.path().join("other"), b"content")?;
    let gate = Gate::default();
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(Op::Stat, "stuck", 0, gate.racer())],
            ..Quirks::default()
        },
    );
    let stuck = Location::local(root.path().join("stuck"));
    let parked =
        std::thread::spawn(move || answer(presence(&[stuck], &gio::Cancellable::new())).len());
    wait_until("the probe reaches the seam", || gate.reached());
    wait_until("the folder stalls", || {
        mount_lanes::responsiveness(root.path()) == mount_lanes::Responsiveness::Stalled
    });

    let started = Instant::now();
    let other = answer(presence(
        &[Location::local(root.path().join("other"))],
        &gio::Cancellable::new(),
    ));
    let listed = answer(folder(root.path()));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(
        matches!(other.as_slice(), [Presence::Unknown(error)] if error.kind() == io::ErrorKind::TimedOut),
        "{other:?}"
    );
    assert!(
        matches!(listed, Err(LocationValidationError::Unavailable(_))),
        "{listed:?}"
    );

    drop(gate);
    assert_eq!(parked.join().expect("parked probe"), 1);
    assert!(matches!(
        answer(presence(
            &[Location::local(root.path().join("other"))],
            &gio::Cancellable::new(),
        ))
        .as_slice(),
        [Presence::Present(EntryKind::File)]
    ));
    Ok(())
}

#[test]
fn presence_waits_on_one_mount_with_one_worker_however_many_folders_it_asks_about()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let mut locations = Vec::new();
    for folder in ["a", "b", "c", "d", "e", "f"] {
        fs::create_dir(root.path().join(folder))?;
        fs::write(root.path().join(folder).join("file"), b"content")?;
        locations.push(Location::local(root.path().join(folder).join("file")));
    }
    let gate = Gate::default();
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(Op::Stat, "file", 0, gate.racer())],
            ..Quirks::default()
        },
    );

    let parked =
        std::thread::spawn(move || answer(presence(&locations, &gio::Cancellable::new())).len());
    wait_until("the probe reaches the seam", || gate.reached());
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        scope.count(Op::Stat),
        1,
        "the other folders wait their turn"
    );

    drop(gate);
    assert_eq!(parked.join().expect("parked probe"), 6);
    assert_eq!(scope.count(Op::Stat), 6);
    Ok(())
}

#[test]
fn a_folder_checked_again_while_its_check_is_parked_waits_for_one_more_call()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let slow = root.path().join("slow");
    fs::create_dir(&slow)?;
    let gate = Gate::default();
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(Op::FolderStamp, "slow", 0, gate.racer())],
            ..Quirks::default()
        },
    );
    let check = |slow: &std::path::Path| {
        let slow = slow.to_owned();
        std::thread::spawn(move || answer(folder(&slow)))
    };

    let first = check(&slow);
    wait_until("the check reaches the seam", || gate.reached());
    let repeats = [check(&slow), check(&slow)];
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(scope.count(Op::FolderStamp), 1, "the repeats wait");

    drop(gate);
    for check in std::iter::once(first).chain(repeats) {
        assert_eq!(check.join().expect("check"), Ok(()));
    }
    assert_eq!(scope.count(Op::FolderStamp), 2);
    Ok(())
}
