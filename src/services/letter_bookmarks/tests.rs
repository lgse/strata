// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn letter_bookmarks_reload_replace_and_preserve_other_slots() {
    let root = tempfile::tempdir().expect("temporary directory");
    let store = root.path().join("state/marks.json");
    assert_eq!(read(&store, 'a').expect("missing store"), None);
    assert!(read_all(&store).expect("empty menu").is_empty());
    write(&store, 'a', Path::new("/first folder")).expect("first mark");
    write(&store, 'z', Path::new("/other/#percent%")).expect("other mark");
    write(&store, 'a', Path::new("/replacement")).expect("replace mark");
    assert_eq!(
        read(&store, 'a').expect("reload"),
        Some(PathBuf::from("/replacement"))
    );
    assert_eq!(
        read(&store, 'z').expect("preserved mark"),
        Some(PathBuf::from("/other/#percent%"))
    );
    assert_eq!(read(&store, 'b').expect("unused slot"), None);
    assert_eq!(
        read_all(&store).expect("saved menu entries"),
        vec![
            ('a', PathBuf::from("/replacement")),
            ('z', PathBuf::from("/other/#percent%")),
        ]
    );
}

#[test]
fn letter_bookmarks_preserve_non_utf8_paths() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let root = tempfile::tempdir().expect("temporary directory");
    let store = root.path().join("marks.json");
    let path = PathBuf::from(OsString::from_vec(b"/directory-\xff".to_vec()));
    write(&store, 'a', &path).expect("save byte-safe path");
    assert_eq!(read(&store, 'a').expect("reload"), Some(path));
}

#[test]
fn letter_bookmarks_refuse_invalid_input_and_preserve_corrupt_state() {
    let root = tempfile::tempdir().expect("temporary directory");
    let store = root.path().join("marks.json");
    for (letter, path) in [('A', "/absolute"), ('1', "/absolute"), ('a', "relative")] {
        assert!(write(&store, letter, Path::new(path)).is_err());
        assert!(!store.exists());
    }
    std::fs::write(&store, b"not JSON").expect("corrupt store");
    assert!(read(&store, 'a').is_err());
    assert!(write(&store, 'a', Path::new("/absolute")).is_err());
    assert_eq!(std::fs::read(&store).expect("original data"), b"not JSON");
}

#[test]
fn letter_bookmarks_never_follow_a_symlink_on_save() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().expect("temporary directory");
    let target = root.path().join("original.json");
    let store = root.path().join("marks.json");
    write(&target, 'z', Path::new("/original")).expect("original store");
    let original = std::fs::read(&target).expect("original bytes");
    symlink(&target, &store).expect("symlink store");
    assert!(write(&store, 'a', Path::new("/new")).is_err());
    assert_eq!(std::fs::read(&target).expect("unchanged target"), original);
    assert!(store.is_symlink());
}

#[test]
fn letter_bookmarks_ignore_remote_uri_targets() {
    let root = tempfile::tempdir().expect("temporary directory");
    let store = root.path().join("marks.json");
    std::fs::write(
        &store,
        br#"{"entries":{"a":"sftp://example.test/private","Z":"file:///uppercase","b":"file:///local%20folder"}}"#,
    )
    .expect("remote entry");
    assert_eq!(read(&store, 'a').expect("load remote mark"), None);
    assert_eq!(
        read_all(&store).expect("only usable menu entries"),
        vec![('b', PathBuf::from("/local folder")),]
    );
}
