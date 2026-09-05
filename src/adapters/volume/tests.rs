// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::Path,
    time::{Duration, Instant},
};

use super::*;
use crate::{model::Location, services::VolumeRelation};
use gtk::{gio, prelude::*};

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

#[test]
fn two_subdirs_of_the_same_tempdir_are_the_same_volume() {
    let root = tempfile::tempdir().expect("tempdir");
    let left = root.path().join("left");
    let right = root.path().join("right");
    fs::create_dir(&left).expect("left");
    fs::create_dir(&right).expect("right");
    let left_id =
        query_volume_identity(&Location::local(&left), true, None).expect("left identity");
    let right_id =
        query_volume_identity(&Location::local(&right), true, None).expect("right identity");
    assert!(left_id.matches(&right_id));
}

#[test]
fn native_path_matches_itself() {
    let root = tempfile::tempdir().expect("tempdir");
    let location = Location::local(root.path());
    let first = query_volume_identity(&location, true, None).expect("identity");
    let second = query_volume_identity(&location, true, None).expect("identity");
    assert!(first.matches(&second));
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
    let dest_id = query_volume_identity(&Location::local(&link), true, None).expect("dest follows");
    let source_id =
        query_volume_identity(&Location::local(&source), false, None).expect("source lstat");
    assert!(
        !dest_id.matches(&source_id),
        "followed dest {} should differ from source {}",
        dest_id.filesystem_id,
        source_id.filesystem_id
    );
}

#[test]
fn file_uri_query_uses_gio_filesystem_id() {
    let root = tempfile::tempdir().expect("tempdir");
    let uri = gio::File::for_path(root.path()).uri();
    let location = Location::uri(uri.to_string());
    let identity = query_volume_identity(&location, true, None).expect("file uri identity");
    assert!(!identity.filesystem_id.is_empty());
    assert!(!identity.is_remote);
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
fn hover_skips_uri_lookups() {
    let dest = Location::uri("smb://host/share");
    let source = Location::uri("smb://host/share/file");
    let hover = lookup_drop_volumes(Some(&dest), std::slice::from_ref(&source), false);
    assert!(hover.dest.is_none());
    assert!(hover.sources.iter().all(Option::is_none));
    assert_eq!(hover.relation, VolumeRelation::Unknown);
}

#[test]
fn hover_stats_native_paths() {
    let root = tempfile::tempdir().expect("tempdir");
    let dest = Location::local(root.path());
    let file = root.path().join("file");
    fs::write(&file, b"x").expect("file");
    let source = Location::local(&file);
    let hover = lookup_drop_volumes(Some(&dest), std::slice::from_ref(&source), false);
    assert_eq!(hover.relation, VolumeRelation::Same);
}

#[test]
fn cancelled_cancellable_skips_uri_query() {
    let root = tempfile::tempdir().expect("tempdir");
    let uri = gio::File::for_path(root.path()).uri();
    let location = Location::uri(uri.to_string());
    let cancellable = gio::Cancellable::new();
    cancellable.cancel();
    assert!(query_volume_identity(&location, true, Some(&cancellable)).is_none());
}

#[test]
fn native_query_ignores_cancelled_cancellable() {
    let root = tempfile::tempdir().expect("tempdir");
    let location = Location::local(root.path());
    let cancellable = gio::Cancellable::new();
    cancellable.cancel();
    assert!(query_volume_identity(&location, true, Some(&cancellable)).is_some());
}

#[test]
fn watchdog_cancels_after_timeout() {
    let cancellable = cancellable_with_timeout(Duration::from_millis(30));
    assert!(!cancellable.is_cancelled());
    let started = Instant::now();
    while !cancellable.is_cancelled() {
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "volume query watchdog did not cancel"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
