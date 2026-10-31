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
fn letter_bookmarks_refuse_invalid_input() {
    let root = tempfile::tempdir().expect("temporary directory");
    let store = root.path().join("marks.json");
    for (letter, path) in [('A', "/absolute"), ('1', "/absolute"), ('a', "relative")] {
        assert!(write(&store, letter, Path::new(path)).is_err());
        assert!(!store.exists());
    }
}

#[test]
fn letter_bookmarks_save_over_a_file_that_no_longer_parses() {
    let root = tempfile::tempdir().expect("temporary directory");
    let store = root.path().join("marks.json");
    for corrupt in [&b"not JSON"[..], b"{\"entries\":{\"a\":", b"{\"entries\":[]}"] {
        std::fs::write(&store, corrupt).expect("corrupt store");
        assert!(read(&store, 'a').is_err());
        assert!(read_all(&store).is_err());
        write(&store, 'b', Path::new("/recovered")).expect("save replaces corrupt store");
        assert_eq!(
            read_all(&store).expect("recovered store"),
            vec![('b', PathBuf::from("/recovered"))]
        );
    }
}

#[test]
fn letter_bookmarks_load_files_with_unknown_fields_and_keys() {
    let root = tempfile::tempdir().expect("temporary directory");
    let store = root.path().join("marks.json");
    std::fs::write(
        &store,
        br#"{"version":2,"entries":{"a":"file:///kept","ab":"file:///long-key","":"file:///empty"}}"#,
    )
    .expect("newer store");
    assert_eq!(read(&store, 'a').expect("known slot"), Some(PathBuf::from("/kept")));
    assert_eq!(
        read_all(&store).expect("usable entries"),
        vec![('a', PathBuf::from("/kept"))]
    );
    write(&store, 'c', Path::new("/added")).expect("save beside unknown data");
    assert_eq!(
        read_all(&store).expect("saved store"),
        vec![('a', PathBuf::from("/kept")), ('c', PathBuf::from("/added"))]
    );
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
    assert_eq!(
        read(&store, 'a').expect_err("invalid remote mark").kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        read_all(&store).expect("only usable menu entries"),
        vec![('b', PathBuf::from("/local folder")),]
    );
    write(&store, 'c', Path::new("/new folder")).expect("repair during save");
    let stored = load(&store).expect("repaired file");
    assert_eq!(
        stored.entries.keys().cloned().collect::<Vec<_>>(),
        vec!["b", "c"]
    );
    assert_eq!(read(&store, 'a').expect("removed remote entry"), None);
}
