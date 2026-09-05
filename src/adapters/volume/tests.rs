// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::RefCell,
    fs,
    os::unix::fs::MetadataExt,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

use super::*;
use crate::{model::Location, services::VolumeRelation};
use gtk::{gio, glib};

fn distinct_device_dirs() -> Option<(tempfile::TempDir, tempfile::TempDir)> {
    let first = tempfile::tempdir().ok()?;
    let shm = Path::new("/dev/shm");
    if !shm.is_dir() {
        return None;
    }
    let second = tempfile::TempDir::new_in(shm).ok()?;
    let first_dev = fs::metadata(first.path()).ok()?.dev();
    let second_dev = fs::metadata(second.path()).ok()?.dev();
    (first_dev != second_dev).then_some((first, second))
}

fn ready(volumes: DropVolumes) -> DropVolumeLookup {
    match volumes {
        DropVolumes::Ready(lookup) => lookup,
        DropVolumes::Pending(_) => panic!("native lookup should resolve synchronously"),
    }
}

/// Drives a URI lookup on a private main context until `on_ready` fires or
/// `deadline` passes, returning the lookup and whether it was pending at first.
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
    let query = DropVolumeQuery::new(&dest, &[Location::local("/")]);
    assert_eq!(query.source_parents, vec![Location::local("/")]);
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
    let lookup = ready(lookup_drop_volumes(&query, |_| {}));
    assert_eq!(lookup.relation, VolumeRelation::Same);
}

#[test]
fn native_lookup_is_ready_and_never_calls_on_ready() {
    let root = tempfile::tempdir().expect("tempdir");
    let called = Rc::new(std::cell::Cell::new(false));
    let flag = called.clone();
    let query = DropVolumeQuery::new(
        &Location::local(root.path()),
        &[Location::local(root.path())],
    );
    let lookup = ready(lookup_drop_volumes(&query, move |_| flag.set(true)));
    assert_eq!(lookup.relation, VolumeRelation::Same);
    assert!(!called.get());
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
    let lookup = ready(lookup_drop_volumes(&query, |_| {}));
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
    let lookup = ready(lookup_drop_volumes(&query, |_| {}));
    assert_eq!(lookup.relation, VolumeRelation::Same);
}

#[test]
fn missing_native_directory_is_unknown() {
    let root = tempfile::tempdir().expect("tempdir");
    let query = DropVolumeQuery::new(
        &Location::local(root.path().join("missing")),
        &[Location::local(root.path().join("file"))],
    );
    let lookup = ready(lookup_drop_volumes(&query, |_| {}));
    assert!(lookup.dest.is_none());
    assert_eq!(lookup.relation, VolumeRelation::Unknown);
}

#[test]
fn smb_and_sftp_uris_are_remote() {
    assert!(!location_is_remote(&Location::local("/tmp")));
    assert!(!location_is_remote(&Location::uri("trash:///foo")));
    assert!(!location_is_remote(&Location::uri("file:///tmp")));
    assert!(location_is_remote(&Location::uri("smb://host/share")));
    assert!(location_is_remote(&Location::uri("sftp://host/path")));
}

#[test]
fn uri_lookup_is_pending_then_reports_once_resolved() {
    let root = tempfile::tempdir().expect("tempdir");
    let file = root.path().join("file");
    fs::write(&file, b"x").expect("file");
    let dest = Location::uri(gio::File::for_path(root.path()).uri().to_string());
    let source = Location::uri(gio::File::for_path(&file).uri().to_string());
    let query = DropVolumeQuery::new(&dest, &[source]);
    assert!(!query.is_native());
    let (lookup, was_pending) = resolve_on_private_context(&query, Duration::from_secs(5));
    assert!(was_pending);
    let lookup = lookup.expect("file uri lookup should resolve");
    assert_eq!(lookup.relation, VolumeRelation::Same);
    assert!(
        lookup
            .dest
            .is_some_and(|identity| { !identity.filesystem_id.is_empty() && !identity.is_remote })
    );
}

#[test]
fn native_path_and_file_uri_of_the_same_directory_share_an_identity() {
    let root = tempfile::tempdir().expect("tempdir");
    let file = root.path().join("file");
    fs::write(&file, b"x").expect("file");
    let native = Location::local(root.path());
    let uri = Location::uri(gio::File::for_path(&file).uri().to_string());
    let query = DropVolumeQuery::new(&native, &[uri]);
    assert!(!query.is_native());
    let (lookup, was_pending) = resolve_on_private_context(&query, Duration::from_secs(5));
    assert!(was_pending);
    let lookup = lookup.expect("mixed lookup should resolve");
    assert_eq!(
        lookup.relation,
        VolumeRelation::Same,
        "native and file:// ids must use one encoding: {}",
        lookup.describe()
    );
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
                    // Mirrors the UI cache, which swaps the Pending handle for
                    // the Ready result while the resolver is still on the stack.
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
    let (lookup, was_pending) =
        resolve_on_private_context(&query, REMOTE_QUERY_TIMEOUT + Duration::from_secs(3));
    assert!(was_pending);
    let lookup = lookup.expect("failed or timed out remote lookup should still report");
    assert!(lookup.dest.is_none());
    assert_eq!(lookup.relation, VolumeRelation::Unknown);
}
