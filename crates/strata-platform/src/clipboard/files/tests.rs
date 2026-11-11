use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::{Files, GNOME_COPIED_FILES, KDE_CUT_SELECTION, URI_LIST};
use crate::clipboard::Content;

fn payloads(files: &Files) -> Vec<(String, Vec<u8>)> {
    let (spec, _) = files.add_to(Content::new()).into_parts();
    spec.mimes()
        .map(|m| {
            let bytes = spec.writer(m).and_then(|w| w.get()).expect("bytes");
            (m.to_owned(), bytes.to_vec())
        })
        .collect()
}

#[test]
fn a_copy_writes_crlf_uri_list_a_copy_header_with_no_trailing_newline_and_kde_0() {
    let files = Files::new(["file:///tmp/a", "file:///tmp/b"], false);
    assert_eq!(
        payloads(&files),
        [
            (
                GNOME_COPIED_FILES.to_owned(),
                b"copy\nfile:///tmp/a\nfile:///tmp/b".to_vec()
            ),
            (
                URI_LIST.to_owned(),
                b"file:///tmp/a\r\nfile:///tmp/b\r\n".to_vec()
            ),
            (KDE_CUT_SELECTION.to_owned(), b"0".to_vec()),
        ]
    );
}

#[test]
fn a_cut_writes_the_cut_header_and_kde_1() {
    let p = payloads(&Files::new(["file:///tmp/a"], true));
    assert_eq!(p[0].1, b"cut\nfile:///tmp/a");
    assert_eq!(p[2].1, b"1");
}

#[test]
fn paths_that_arent_utf8_or_hold_reserved_bytes_round_trip_through_uris() {
    let paths = [
        PathBuf::from(OsStr::from_bytes(b"/tmp/bad\xff.txt")),
        PathBuf::from("/tmp/a b/100%#?.txt"),
        PathBuf::from("/tmp/line\nbreak"),
    ];
    let files = Files::from_paths(&paths, false);
    assert_eq!(files.uris[0], "file:///tmp/bad%FF.txt");
    assert!(
        files
            .uris
            .iter()
            .all(|u| u.is_ascii() && !u.contains(['\n', ' ', '#']))
    );
    let p = payloads(&files);
    let back = Files::decode(Some(&p[0].1), None, None).expect("files");
    assert_eq!(back.paths(), paths);
}

#[test]
fn the_gnome_header_decides_whatever_the_kde_flag_says() {
    let f = Files::decode(Some(b"cut\nfile:///a"), Some(b"file:///a\r\n"), Some(b"0"));
    assert_eq!(f, Some(Files::new(["file:///a"], true)));
    let f = Files::decode(Some(b"copy\nfile:///a"), Some(b"file:///a\r\n"), Some(b"1"));
    assert_eq!(f, Some(Files::new(["file:///a"], false)));
}

#[test]
fn without_a_valid_gnome_payload_the_uri_list_and_the_kde_flag_decide() {
    for gnome in [
        None,
        Some(&b"CUT\nfile:///x"[..]),
        Some(b"move\nfile:///x"),
        Some(b"cut"),
    ] {
        assert_eq!(
            Files::decode(gnome, Some(b"file:///a\r\n"), Some(b"1")),
            Some(Files::new(["file:///a"], true)),
            "{gnome:?}"
        );
        assert_eq!(
            Files::decode(gnome, Some(b"file:///a\r\n"), Some(b"0")),
            Some(Files::new(["file:///a"], false))
        );
        assert_eq!(
            Files::decode(gnome, Some(b"file:///a\r\n"), None),
            Some(Files::new(["file:///a"], false)),
            "no KDE flag is a copy"
        );
    }
}

#[test]
fn crlf_a_trailing_newline_and_nul_padding_read_as_plain_newlines() {
    let want = Some(Files::new(["file:///a", "file:///b"], true));
    for payload in [
        &b"cut\r\nfile:///a\r\nfile:///b"[..],
        b"cut\nfile:///a\nfile:///b\n",
        b"cut\nfile:///a\nfile:///b\0",
    ] {
        assert_eq!(
            Files::decode(Some(payload), None, None),
            want,
            "{payload:?}"
        );
    }
}

#[test]
fn a_gnome_payload_alone_still_names_the_files() {
    assert_eq!(
        Files::decode(Some(b"copy\nfile:///a"), None, None),
        Some(Files::new(["file:///a"], false))
    );
}

#[test]
fn nothing_that_names_a_file_is_no_file_list() {
    assert_eq!(Files::decode(None, None, Some(b"1")), None);
    assert_eq!(
        Files::decode(Some(b"copy"), Some(b"# a comment\r\n"), None),
        None
    );
}

#[test]
fn files_are_offered_by_the_gnome_type_or_the_uri_list() {
    assert!(Files::offered(&[GNOME_COPIED_FILES]));
    assert!(Files::offered(&[URI_LIST]));
    assert!(!Files::offered(&[KDE_CUT_SELECTION, "text/plain"]));
}

#[test]
fn paths_leave_out_other_schemes_and_hosts() {
    let f = Files::new(["file:///a", "smb://host/share/b", "file://other/c"], false);
    assert_eq!(f.paths(), [Path::new("/a")]);
}
