// SPDX-License-Identifier: GPL-3.0-or-later

use std::{fs, os::unix::fs::MetadataExt, path::Path, time::SystemTime};

use super::*;
use crate::model::Location;

fn unique_root(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("strata-volume-{label}-{unique}"))
}

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

fn sample_identity() -> VolumeIdentity {
    VolumeIdentity {
        filesystem_id: "dev:test".into(),
        is_remote: false,
    }
}

#[test]
fn two_subdirs_of_the_same_tempdir_are_the_same_volume() {
    let root = tempfile::tempdir().expect("tempdir");
    let left = root.path().join("left");
    let right = root.path().join("right");
    fs::create_dir(&left).expect("left");
    fs::create_dir(&right).expect("right");
    let left_id = query_volume_identity(&Location::local(&left), true).expect("left identity");
    let right_id = query_volume_identity(&Location::local(&right), true).expect("right identity");
    assert!(left_id.matches(&right_id));
}

#[test]
fn native_path_matches_itself() {
    let root = tempfile::tempdir().expect("tempdir");
    let location = Location::local(root.path());
    let first = query_volume_identity(&location, true).expect("identity");
    let second = query_volume_identity(&location, true).expect("identity");
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
    let dest_id = query_volume_identity(&Location::local(&link), true).expect("dest follows");
    let source_id = query_volume_identity(&Location::local(&source), false).expect("source lstat");
    assert!(
        !dest_id.matches(&source_id),
        "followed dest {} should differ from source {}",
        dest_id.filesystem_id,
        source_id.filesystem_id
    );
}

#[test]
fn missing_identity_is_not_cached() {
    let root = unique_root("missing");
    let location = Location::local(root.join("later"));
    assert!(query_volume_identity(&location, true).is_none());
    assert!(cached_volume_identity(&location, true).is_none());
    fs::create_dir_all(root.join("later")).expect("create");
    assert!(query_volume_identity(&location, true).is_some());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn flush_clears_entries_and_inflight_and_bumps_epoch() {
    let root = tempfile::tempdir().expect("tempdir");
    let location = Location::local(root.path());
    assert!(query_volume_identity(&location, true).is_some());
    assert!(cached_volume_identity(&location, true).is_some());
    let before = volume_epoch();
    flush_volume_identity_cache();
    assert!(cached_volume_identity(&location, true).is_none());
    assert_eq!(volume_cache_len(), 0);
    assert_eq!(volume_inflight_len(), 0);
    assert_ne!(volume_epoch(), before);
}

#[test]
fn inflight_success_after_flush_does_not_reenter_the_lru() {
    flush_volume_identity_cache();
    let key = CacheKey {
        location: Location::uri("smb://host/share"),
        follow_symlinks: true,
    };
    let epoch = begin_inflight(&key).expect("first inflight");
    assert_eq!(volume_inflight_len(), 1);
    flush_volume_identity_cache();
    assert_eq!(volume_inflight_len(), 0);
    complete_inflight(key.clone(), epoch, Some(sample_identity()));
    assert!(cached_volume_identity(&key.location, true).is_none());
}

#[test]
fn native_query_after_flush_matches_a_fresh_stat() {
    let root = tempfile::tempdir().expect("tempdir");
    let location = Location::local(root.path());
    let before = query_volume_identity(&location, true).expect("before");
    flush_volume_identity_cache();
    let after = query_volume_identity(&location, true).expect("after flush");
    assert!(before.matches(&after));
}

#[test]
fn prefetch_coalesces_duplicate_uri_lookups() {
    flush_volume_identity_cache();
    let key = CacheKey {
        location: Location::uri("smb://host/share"),
        follow_symlinks: true,
    };
    assert!(begin_inflight(&key).is_some());
    assert!(begin_inflight(&key).is_none());
    assert_eq!(volume_inflight_len(), 1);
}

#[test]
fn file_uri_query_uses_gio_filesystem_id() {
    let root = tempfile::tempdir().expect("tempdir");
    let uri = gio::File::for_path(root.path()).uri();
    let location = Location::uri(uri.to_string());
    let identity = query_volume_identity(&location, true).expect("file uri identity");
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
