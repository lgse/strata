use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::*;

fn path(bytes: &[u8]) -> &Path {
    Path::new(OsStr::from_bytes(bytes))
}

/// The table is what GLib's `g_filename_to_uri` gave for each path (Python's
/// `GLib.filename_to_uri`, recorded in P0's handoff).
#[test]
fn file_uris_escape_like_glib() {
    let cases: &[(&[u8], &str)] = &[
        (b"/tmp/a b.txt", "file:///tmp/a%20b.txt"),
        (b"/tmp/100%.txt", "file:///tmp/100%25.txt"),
        (b"/tmp/#hash?q.txt", "file:///tmp/%23hash%3Fq.txt"),
        (b"/tmp/caf\xc3\xa9", "file:///tmp/caf%C3%A9"),
        (b"/tmp/\xff\xfe-raw", "file:///tmp/%FF%FE-raw"),
        (b"/tmp/!$&'()*+,;=:@~-._", "file:///tmp/!$&'()*+,%3B=:@~-._"),
        (
            b"/tmp/[x]{y}<z>|\\^`\"",
            "file:///tmp/%5Bx%5D%7By%7D%3Cz%3E%7C%5C%5E%60%22",
        ),
        (b"/tmp/tab\there", "file:///tmp/tab%09here"),
        (b"/tmp/new\nline", "file:///tmp/new%0Aline"),
    ];
    for (p, uri) in cases {
        assert_eq!(file_uri(path(p)), *uri, "{:?}", OsStr::from_bytes(p));
    }
}

#[test]
fn non_utf8_paths_round_trip_through_their_uris() {
    for p in [
        &b"/tmp/\xff\xfe-raw"[..],
        b"/tmp/100%25 not an escape",
        b"/tmp/caf\xc3\xa9/\x80",
        b"/",
    ] {
        let uri = file_uri(path(p));
        assert_eq!(
            file_path(&uri),
            Ok(FileUri::Local(path(p).to_owned())),
            "{uri}"
        );
    }
}

#[test]
fn uri_lists_end_every_line_in_crlf() {
    assert_eq!(
        encode_uri_list(&["file:///a", "file:///b%20c"]),
        b"file:///a\r\nfile:///b%20c\r\n"
    );
    assert_eq!(encode_uri_list::<&str>(&[]), b"");
}

#[test]
fn uri_list_decoding_skips_comments_blank_lines_and_takes_lf() {
    let payload = b"# a comment\r\nfile:///a\r\n\r\nfile:///b\nhttps://example.com/x\n\0\0";
    assert_eq!(
        decode_uri_list(payload),
        ["file:///a", "file:///b", "https://example.com/x"]
    );
}

#[test]
fn file_uri_decoding_rejects_bad_and_slash_escapes() {
    assert_eq!(file_path("file:///tmp/%zz"), Err(UriError::BadEscape));
    assert_eq!(file_path("file:///tmp/a%"), Err(UriError::BadEscape));
    assert_eq!(file_path("file:///tmp/a%2"), Err(UriError::BadEscape));
    assert_eq!(
        file_path("file:///tmp/%2F"),
        Err(UriError::EscapedSeparator)
    );
    assert_eq!(
        file_path("file:///tmp/%00"),
        Err(UriError::EscapedSeparator)
    );
    assert_eq!(file_path("file:relative"), Err(UriError::NotAUri));
    assert_eq!(file_path("/tmp/no-scheme"), Err(UriError::NotAUri));
}

#[test]
fn file_uri_decoding_takes_localhost_and_the_short_form_and_reports_other_hosts() {
    let local = |p: &str| Ok(FileUri::Local(PathBuf::from(p)));
    assert_eq!(file_path("file:///tmp/a"), local("/tmp/a"));
    assert_eq!(file_path("file://localhost/tmp/a"), local("/tmp/a"));
    assert_eq!(file_path("FILE:///tmp/a"), local("/tmp/a"));
    assert_eq!(file_path("file:/tmp/a"), local("/tmp/a"));
    // Qt leaves more unescaped than GLib; it still reads.
    assert_eq!(file_path("file:///tmp/a;b[1]"), local("/tmp/a;b[1]"));
    assert_eq!(file_path("file:///tmp/a#frag"), local("/tmp/a"));
    assert_eq!(
        file_path("file://host/tmp/a"),
        Ok(FileUri::Remote {
            host: "host".into(),
            path: PathBuf::from("/tmp/a")
        })
    );
    assert_eq!(
        file_path("smb://host/share/a"),
        Ok(FileUri::Other("smb://host/share/a".into()))
    );
}

#[test]
fn gnome_copied_files_has_no_trailing_newline() {
    assert_eq!(
        encode_gnome_copied_files(false, &["file:///a", "file:///b"]),
        b"copy\nfile:///a\nfile:///b"
    );
    assert_eq!(
        encode_gnome_copied_files(true, &["file:///a"]),
        b"cut\nfile:///a"
    );
}

#[test]
fn gnome_copied_files_reads_crlf_a_trailing_newline_and_nul() {
    let want = Some((true, vec!["file:///a".to_owned(), "file:///b".to_owned()]));
    assert_eq!(
        decode_gnome_copied_files(b"cut\nfile:///a\nfile:///b"),
        want
    );
    assert_eq!(
        decode_gnome_copied_files(b"cut\r\nfile:///a\r\nfile:///b\r\n"),
        want
    );
    assert_eq!(
        decode_gnome_copied_files(b"cut\nfile:///a\nfile:///b\n\0"),
        want
    );
    assert_eq!(
        decode_gnome_copied_files(b"copy\nfile:///a"),
        Some((false, vec!["file:///a".to_owned()]))
    );
}

#[test]
fn gnome_copied_files_ignores_an_unknown_action_or_no_files() {
    assert_eq!(decode_gnome_copied_files(b"move\nfile:///a"), None);
    assert_eq!(decode_gnome_copied_files(b"copy"), None);
    assert_eq!(decode_gnome_copied_files(b"copy\n\n"), None);
    assert_eq!(decode_gnome_copied_files(b""), None);
}

#[test]
fn kde_cut_reads_the_first_byte() {
    assert_eq!(encode_kde_cut(true), b"1");
    assert_eq!(encode_kde_cut(false), b"0");
    assert!(decode_kde_cut(b"1"));
    assert!(decode_kde_cut(b"1\n"));
    assert!(decode_kde_cut(b"1\0"));
    assert!(!decode_kde_cut(b"0"));
    assert!(!decode_kde_cut(b""));
}

#[test]
fn utf8_text_aliases_utf8_string() {
    assert_eq!(x11_aliases("text/plain;charset=utf-8"), ["UTF8_STRING"]);
    assert_eq!(x11_aliases("text/plain; charset=UTF-8"), ["UTF8_STRING"]);
    assert_eq!(x11_aliases("text/plain"), ["STRING", "TEXT"]);
    assert!(x11_aliases(URI_LIST).is_empty());
}

#[test]
fn pixmap_is_data_and_meta_targets_match_by_exact_name() {
    for name in ["PIXMAP", "targets", "Save_Targets", "SAVE_TARGETS "] {
        assert!(!super::is_x11_meta(name), "{name}");
    }
}
