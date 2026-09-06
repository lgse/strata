// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn local_conversion_preserves_native_path_bytes() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt, path::Path};
    let path = Path::new(OsStr::from_bytes(b"/fixture/non-utf8-\xff"));
    let location = Location::local(path);
    let file = gio_file_for_location(&location);
    assert_eq!(file.path().as_deref(), Some(path));
    assert_eq!(location_for_file(&file), Some(location));
}

#[test]
fn remote_conversion_preserves_gio_identity() {
    for uri in [
        "smb://host/share",
        "sftp://host/path%20with%20spaces",
        "trash:///",
    ] {
        let file = gio_file_for_location(&Location::uri(uri));
        assert!(file.equal(&gio::File::for_uri(uri)));
        let round_trip = location_for_file(&file).expect("location");
        assert!(gio_file_for_location(&round_trip).equal(&file));
    }
}

#[test]
fn native_files_are_located_by_their_real_path() {
    let file = gio::File::for_path("/tmp");
    assert_eq!(location_for_file(&file), Some(Location::local("/tmp")));
}

#[test]
fn gvfs_backed_files_use_their_uri_even_when_a_fuse_path_exists() {
    let file = gio::File::for_uri("smb://host/share");
    assert!(!file.is_native(), "smb:// should never be reported native");
    assert_eq!(location_for_file(&file), Some(Location::uri(file.uri())));
}

#[test]
fn gio_files_with_embedded_credentials_are_sanitized() {
    for uri in [
        "smb://user%3Asecret@host/share",
        "smb://user;password=secret@host/share",
        "smb://user%3Bpassword=secret@host/share",
        "smb://user:secret@host/share",
    ] {
        let location = location_for_file(&gio::File::for_uri(uri))
            .expect("credential URI should produce a sanitized location");
        assert_eq!(
            location
                .uri_value()
                .expect("remote location should have a URI")
                .trim_end_matches('/'),
            "smb://user@host/share",
            "did not sanitize {uri}"
        );
    }
}
