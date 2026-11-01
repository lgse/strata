// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    fs,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

use super::*;
use crate::{model::Location, services::VolumeRelation};
use gtk::{gio, glib};

mod lookup_regressions;

fn distinct_device_dirs() -> Option<(tempfile::TempDir, tempfile::TempDir)> {
    crate::test_support::distinct_device_dirs("volume identity tests")
}

fn resolve_on_private_context(
    query: &DropVolumeQuery,
    deadline: Duration,
) -> (Option<DropVolumeLookup>, bool) {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let result = Rc::new(RefCell::new(None));
            let sink = result.clone();
            let volumes = lookup_drop_volumes(query, move |lookup| {
                *sink.borrow_mut() = Some(lookup);
            });
            let was_pending = matches!(volumes, DropVolumes::Pending(_));
            let started = Instant::now();
            while result.borrow().is_none() && started.elapsed() < deadline {
                context.iteration(false);
                std::thread::sleep(Duration::from_millis(1));
            }
            drop(volumes);
            (result.take(), was_pending)
        })
        .expect("private main context should be acquirable")
}

#[test]
fn query_dedupes_source_parents() {
    let root = tempfile::tempdir().expect("tempdir");
    let dest = Location::local(root.path().join("dest"));
    let sources = [
        Location::local(root.path().join("a/one")),
        Location::local(root.path().join("a/two")),
        Location::local(root.path().join("b/three")),
    ];
    let query = DropVolumeQuery::new(&dest, &sources);
    assert_eq!(query.dest, dest);
    assert_eq!(
        query.source_parents,
        vec![
            Location::local(root.path().join("a")),
            Location::local(root.path().join("b")),
        ]
    );
}

#[test]
fn query_falls_back_to_the_source_when_it_has_no_parent() {
    let dest = Location::local("/tmp");
    let mounts = MountTable::parse("1 0 0:1 / /home rw - ext4 /dev/root rw\n");
    let query = DropVolumeQuery::with_mounts(&dest, &[Location::local("/")], &mounts);
    assert_eq!(query.source_parents, vec![Location::local("/")]);
}

#[test]
fn mount_point_source_is_queried_instead_of_its_parent() {
    let dest = Location::local("/home/user/folder");
    let usb = Location::local("/run/media/user/USB");
    let file_on_usb = Location::local("/run/media/user/USB/docs/file");
    let mounts = MountTable::parse(
        "1 0 0:1 / / rw - ext4 /dev/root rw\n\
         2 1 8:17 / /run/media/user/USB rw - vfat /dev/sdb1 rw\n",
    );
    let query = DropVolumeQuery::with_mounts(&dest, std::slice::from_ref(&usb), &mounts);
    assert_eq!(query.source_parents, vec![usb]);

    let query = DropVolumeQuery::with_mounts(&dest, &[file_on_usb], &mounts);
    assert_eq!(
        query.source_parents,
        vec![Location::local("/run/media/user/USB/docs")]
    );
}

#[test]
fn two_subdirs_of_the_same_tempdir_are_the_same_volume() {
    let root = tempfile::tempdir().expect("tempdir");
    let left = root.path().join("left");
    let right = root.path().join("right");
    fs::create_dir(&left).expect("left");
    fs::create_dir(&right).expect("right");
    fs::write(right.join("file"), b"x").expect("file");
    let query = DropVolumeQuery::new(
        &Location::local(&left),
        &[Location::local(right.join("file"))],
    );
    let (lookup, was_pending) = resolve_on_private_context(&query, Duration::from_secs(5));
    assert!(
        was_pending,
        "a local folder is never looked up on the caller"
    );
    assert_eq!(
        lookup.expect("local lookup should resolve").relation,
        VolumeRelation::Same
    );
}

#[test]
fn dest_directory_symlink_to_another_device_is_different() {
    let Some((home, stick)) = distinct_device_dirs() else {
        return;
    };
    let source = home.path().join("file");
    fs::write(&source, b"x").expect("source");
    let link = home.path().join("usb");
    std::os::unix::fs::symlink(stick.path(), &link).expect("symlink");
    let query = DropVolumeQuery::new(&Location::local(&link), &[Location::local(&source)]);
    let (lookup, _) = resolve_on_private_context(&query, Duration::from_secs(5));
    let lookup = lookup.expect("dest symlink lookup should resolve");
    assert_eq!(
        lookup.relation,
        VolumeRelation::Different,
        "followed dest {:?} should differ from source {:?}",
        lookup.dest,
        lookup.sources
    );
}

#[test]
fn source_symlink_to_another_device_stays_on_its_parent_volume() {
    let Some((home, stick)) = distinct_device_dirs() else {
        return;
    };
    let target = stick.path().join("file");
    fs::write(&target, b"x").expect("target");
    let link = home.path().join("link");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    let dest = home.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let query = DropVolumeQuery::new(&Location::local(&dest), &[Location::local(&link)]);
    let (lookup, _) = resolve_on_private_context(&query, Duration::from_secs(5));
    assert_eq!(
        lookup.expect("source link lookup should resolve").relation,
        VolumeRelation::Same
    );
}

#[test]
fn missing_native_directory_is_unknown() {
    let root = tempfile::tempdir().expect("tempdir");
    let query = DropVolumeQuery::new(
        &Location::local(root.path().join("missing")),
        &[Location::local(root.path().join("file"))],
    );
    let (lookup, _) = resolve_on_private_context(&query, Duration::from_secs(5));
    let lookup = lookup.expect("a missing folder still reports");
    assert!(lookup.dest.is_none());
    assert_eq!(lookup.relation, VolumeRelation::Unknown);
}

#[test]
fn restore_identity_walks_missing_dest_to_an_existing_ancestor() {
    let root = tempfile::tempdir().expect("tempdir");
    let dest = root.path().join("new/dir/file.txt");
    let walked = volume_identity_of_existing_ancestor(&Location::local(&dest));
    let root_id = volume_identity_of_existing_ancestor(&Location::local(root.path()));
    assert_eq!(walked, root_id);
    assert_eq!(
        restore_volume_relation(
            &Location::local(root.path().join("source")),
            &Location::local(&dest),
        ),
        VolumeRelation::Same
    );
}

#[test]
fn restore_identity_follows_a_parent_symlink_onto_another_device() {
    let Some((home, stick)) = distinct_device_dirs() else {
        return;
    };
    let source = home.path().join("trashed");
    fs::write(&source, b"x").expect("source");
    let link = home.path().join("usb");
    std::os::unix::fs::symlink(stick.path(), &link).expect("symlink");
    let dest = link.join("payload.desktop");
    assert_eq!(
        restore_volume_relation(&Location::local(&source), &Location::local(&dest)),
        VolumeRelation::Different
    );
}

#[test]
fn restore_source_symlink_is_classified_on_its_own_device() {
    let Some((home, stick)) = distinct_device_dirs() else {
        return;
    };
    let target = stick.path().join("elsewhere");
    fs::write(&target, b"x").expect("target");
    let source = home.path().join("trashed-link");
    std::os::unix::fs::symlink(&target, &source).expect("symlink");
    let dest = home.path().join("payload.desktop");
    assert_eq!(
        restore_volume_relation(&Location::local(&source), &Location::local(&dest)),
        VolumeRelation::Same
    );
}

#[test]
fn restore_identity_rejects_a_path_on_another_device() {
    let Some((home, stick)) = distinct_device_dirs() else {
        return;
    };
    let source = home.path().join("trashed");
    fs::write(&source, b"x").expect("source");
    let dest = stick.path().join(".config/autostart/payload.desktop");
    assert_eq!(
        restore_volume_relation(&Location::local(&source), &Location::local(&dest)),
        VolumeRelation::Different
    );
}

#[test]
fn uri_lookup_is_pending_then_reports_once_resolved() {
    let root = tempfile::tempdir().expect("tempdir");
    let file = root.path().join("file");
    fs::write(&file, b"x").expect("file");
    let dest = Location::uri(gio::File::for_path(root.path()).uri().to_string());
    let source = Location::uri(gio::File::for_path(&file).uri().to_string());
    let query = DropVolumeQuery::new(&dest, &[source]);
    let (lookup, _) = resolve_on_private_context(&query, Duration::from_secs(5));
    let lookup = lookup.expect("file uri lookup should resolve");
    assert_eq!(lookup.relation, VolumeRelation::Same);
    assert!(lookup.dest.is_some_and(|identity| {
        !identity.filesystem_id.is_empty() && identity.backend == "file"
    }));
}

#[test]
fn native_path_and_file_uri_of_the_same_directory_share_an_identity() {
    let root = tempfile::tempdir().expect("tempdir");
    let file = root.path().join("file");
    fs::write(&file, b"x").expect("file");
    let native = Location::local(root.path());
    let uri = Location::uri(gio::File::for_path(&file).uri().to_string());
    let query = DropVolumeQuery::new(&native, &[uri]);
    let (lookup, _) = resolve_on_private_context(&query, Duration::from_secs(5));
    let lookup = lookup.expect("mixed lookup should resolve");
    assert_eq!(
        lookup.relation,
        VolumeRelation::Same,
        "native and file:// ids must use one encoding: {}",
        lookup.describe()
    );
}

fn mounts_treating_as(path: &Path, fs_type: &str) -> MountTable {
    let mount_point = path
        .to_str()
        .expect("UTF-8 fixture path")
        .replace('\\', "\\134")
        .replace(' ', "\\040")
        .replace('\t', "\\011")
        .replace('\n', "\\012");
    MountTable::parse(format!(
        "1 0 0:1 / / rw - ext4 /dev/root rw\n\
         2 1 0:2 / {mount_point} rw - {fs_type} fixture rw\n",
    ))
}

#[test]
fn network_mount_aliases_preserve_identity_and_plain_drop_move_policy() {
    use crate::services::{
        CrossVolumeDropStrategy, DropActionInput, DropCommit, DropOverride, drop_commit,
    };

    let root = tempfile::tempdir().expect("tempdir");
    let nas = root.path().join("nas");
    let alias = root.path().join("alias");
    fs::create_dir_all(nas.join("a")).expect("source directory");
    fs::create_dir(nas.join("b")).expect("destination directory");
    fs::write(nas.join("a/file"), b"x").expect("source file");
    std::os::unix::fs::symlink(&nas, &alias).expect("mount alias");
    let mounts = mounts_treating_as(&nas, "nfs4");

    for (source_root, dest_root) in [(&nas, &alias), (&alias, &nas)] {
        let query = DropVolumeQuery::with_mounts(
            &Location::local(dest_root.join("b")),
            &[Location::local(source_root.join("a/file"))],
            &mounts,
        );
        let (lookup, _) = resolve_on_private_context(&query, Duration::from_secs(5));
        let lookup = lookup.expect("alias lookup should resolve");
        let dest = lookup.dest.as_ref().expect("destination identity");
        let source = lookup.sources[0].as_ref().expect("source identity");
        assert_eq!(dest.filesystem_id, source.filesystem_id);
        assert_eq!(lookup.relation, VolumeRelation::Same);
        for strategy in [CrossVolumeDropStrategy::Copy, CrossVolumeDropStrategy::Ask] {
            for (override_with, expected) in [
                (DropOverride::None, DropCommit::Move),
                (DropOverride::ForceCopy, DropCommit::Copy),
                (DropOverride::ForceMove, DropCommit::Move),
            ] {
                assert_eq!(
                    drop_commit(DropActionInput {
                        can_copy: true,
                        can_move: true,
                        volume: lookup.relation,
                        override_with,
                        strategy,
                    }),
                    expected
                );
            }
        }
    }
}

#[test]
fn dropping_the_pending_handle_inside_on_ready_is_safe() {
    let root = tempfile::tempdir().expect("tempdir");
    let file = root.path().join("file");
    fs::write(&file, b"x").expect("file");
    let dest = Location::uri(gio::File::for_path(root.path()).uri().to_string());
    let source = Location::uri(gio::File::for_path(&file).uri().to_string());
    let query = DropVolumeQuery::new(&dest, &[source]);
    let context = glib::MainContext::new();
    let relation = context
        .with_thread_default(|| {
            let slot: Rc<RefCell<Option<DropVolumes>>> = Rc::new(RefCell::new(None));
            let relation = Rc::new(RefCell::new(None));
            let volumes = lookup_drop_volumes(&query, {
                let slot = slot.clone();
                let relation = relation.clone();
                move |lookup| {
                    slot.borrow_mut().take();
                    *relation.borrow_mut() = Some(lookup.relation);
                }
            });
            *slot.borrow_mut() = Some(volumes);
            let started = Instant::now();
            while relation.borrow().is_none() && started.elapsed() < Duration::from_secs(5) {
                context.iteration(false);
                std::thread::sleep(Duration::from_millis(1));
            }
            relation.take()
        })
        .expect("private main context should be acquirable");
    assert_eq!(relation, Some(VolumeRelation::Same));
}

#[test]
fn timeout_finishes_with_partial_results_and_ignores_late_callbacks() {
    let reported = Rc::new(RefCell::new(Vec::new()));
    let sink = reported.clone();
    let state = Rc::new(RefCell::new(PendingState {
        identities: vec![None, None],
        remaining: 2,
        on_ready: Some(Box::new(move |lookup: DropVolumeLookup| {
            sink.borrow_mut().push(lookup);
        })),
        waiting: Vec::new(),
    }));
    let dest = VolumeIdentity {
        filesystem_id: "l1".into(),
        backend: "sftp".into(),
    };
    PendingState::resolve(&state, 0, Some(dest.clone()));
    assert!(reported.borrow().is_empty());

    PendingState::finish(&state);
    {
        let reported = reported.borrow();
        assert_eq!(reported.len(), 1);
        assert_eq!(reported[0].dest.as_ref(), Some(&dest));
        assert_eq!(reported[0].sources, vec![None]);
        assert_eq!(reported[0].relation, VolumeRelation::Unknown);
    }

    PendingState::resolve(&state, 1, Some(dest));
    PendingState::finish(&state);
    assert_eq!(reported.borrow().len(), 1);
}

#[test]
fn cancelled_pending_lookup_never_reports() {
    let root = tempfile::tempdir().expect("tempdir");
    let dest = Location::uri(gio::File::for_path(root.path()).uri().to_string());
    let query = DropVolumeQuery::new(&dest, &[Location::local(root.path())]);
    let context = glib::MainContext::new();
    let reported = context
        .with_thread_default(|| {
            let reported = Rc::new(std::cell::Cell::new(false));
            let flag = reported.clone();
            let volumes = lookup_drop_volumes(&query, move |_| flag.set(true));
            assert_eq!(volumes.relation(), VolumeRelation::Unknown);
            drop(volumes);
            let started = Instant::now();
            while started.elapsed() < Duration::from_millis(200) {
                context.iteration(false);
                std::thread::sleep(Duration::from_millis(1));
            }
            reported.get()
        })
        .expect("private main context should be acquirable");
    assert!(!reported);
}

#[test]
fn unreachable_uri_resolves_to_unknown_without_hanging() {
    let dest = Location::uri("sftp://strata-volume-test.invalid/nowhere");
    let query = DropVolumeQuery::new(&dest, &[Location::local("/tmp")]);
    let (lookup, _) =
        resolve_on_private_context(&query, REMOTE_QUERY_TIMEOUT + Duration::from_secs(3));
    let lookup = lookup.expect("failed or timed out remote lookup should still report");
    assert!(lookup.dest.is_none());
    assert_eq!(lookup.relation, VolumeRelation::Unknown);
}

#[test]
fn navigation_lookups_read_the_mount_table_the_watcher_published() {
    crate::test_support::gtk_test(
        "adapters::volume::tests::navigation_lookups_read_the_mount_table_the_watcher_published",
        || {
            let root = tempfile::tempdir().expect("fixture");
            let share = root.path().join("share");
            let publish = |mountinfo: String| {
                mount_watch::publish_for_isolated_test(Some(MountTable::parse(mountinfo)));
            };
            let looked_up = || {
                with_mount_snapshot(|snapshot| {
                    (
                        snapshot.table.is_network_path(&share.join("folder")),
                        snapshot.table.has_mount_inside(root.path()),
                    )
                })
            };

            publish(format!(
                "1 0 8:1 / / rw - ext4 /dev/root rw\n\
                 900 1 0:90 / {} rw - nfs4 server:/export rw\n",
                share.display()
            ));
            assert_eq!(looked_up(), (true, true));

            publish("1 0 8:1 / / rw - ext4 /dev/root rw\n".to_owned());
            assert_eq!(looked_up(), (false, false));
        },
    );
}
