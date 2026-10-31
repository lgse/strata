// SPDX-License-Identifier: MIT

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
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    for (file, name) in [
        (gio::File::for_uri("smb://host/share"), Some(&b"share"[..])),
        (
            gio::File::for_uri("sftp://host/path%20with%20spaces"),
            Some(&b"path with spaces"[..]),
        ),
        (gio::File::for_uri("trash:///"), None),
        (
            gio::File::for_uri("trash:///").child(OsStr::from_bytes(b"caf\xe9.txt")),
            Some(&b"caf\xe9.txt"[..]),
        ),
        (
            gio::File::for_uri("sftp://host/share").child(OsStr::from_bytes(b"\xff name")),
            Some(&b"\xff name"[..]),
        ),
        (gio::File::for_uri("sftp://host/share/a%2Fb"), None),
        (
            gio::File::for_uri("smb://host/caf%C3%A9"),
            Some("café".as_bytes()),
        ),
    ] {
        let uri = file.uri();
        let round_trip = location_for_file(&file).unwrap_or_else(|| panic!("{uri} has a location"));
        assert!(gio_file_for_location(&round_trip).equal(&file), "{uri}");
        assert_eq!(round_trip, Location::uri(uri.as_str()), "{uri}");
        if let Some(name) = name {
            assert_eq!(
                round_trip.file_name().as_deref().map(OsStrExt::as_bytes),
                Some(name),
                "{uri}"
            );
        }
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
    let location = location_for_file(&gio::File::for_uri("smb://user:secret@host/share"))
        .expect("credential URI should produce a sanitized location");
    assert_eq!(
        location
            .uri_value()
            .expect("remote location should have a URI")
            .trim_end_matches('/'),
        "smb://user@host/share"
    );
}

#[test]
fn reveal_target_for_file_spells_the_child_like_the_listing() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt, path::Path};
    let native = Path::new(OsStr::from_bytes(b"/tmp/x/bad\xe8.txt"));
    assert_eq!(
        reveal_target_for_file(&gio::File::for_path(native)),
        Some((Location::local("/tmp/x"), Location::local(native)))
    );

    let listed = gio::File::for_uri("sftp://host/a%20b").child("c.txt");
    assert_eq!(
        reveal_target_for_file(&gio::File::for_uri("sftp://host/a%20b/c.txt")),
        Some((
            Location::uri("sftp://host/a%20b"),
            Location::uri(listed.uri())
        ))
    );
}
