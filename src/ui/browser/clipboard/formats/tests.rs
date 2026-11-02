// SPDX-License-Identifier: MIT

use super::*;
use gtk::gio;

fn fixture_files() -> Vec<gio::File> {
    vec![
        gio::File::for_path("/fixture/holiday photo.jpg"),
        gio::File::for_uri("sftp://host.example/home/user/video.mp4"),
    ]
}

#[test]
fn copied_files_offer_every_file_manager_format() {
    let formats = file_content(&fixture_files(), false)
        .formats()
        .union_serialize_mime_types();

    for mime in [URI_LIST, GNOME_COPIED_FILES, KDE_CUT_SELECTION] {
        assert!(formats.contain_mime_type(mime), "missing {mime}: {formats}");
    }
    assert!(formats.contains_type(gdk::FileList::static_type()));
}

#[test]
fn gnome_payload_starts_with_the_operation_and_has_no_blank_lines() {
    let files = fixture_files();

    assert_eq!(
        gnome_copied_files(&files, false),
        "copy\nfile:///fixture/holiday%20photo.jpg\nsftp://host.example/home/user/video.mp4"
    );
    assert_eq!(
        gnome_copied_files(&files, true),
        "cut\nfile:///fixture/holiday%20photo.jpg\nsftp://host.example/home/user/video.mp4"
    );
    assert_eq!(
        uri_list(&files),
        "file:///fixture/holiday%20photo.jpg\r\nsftp://host.example/home/user/video.mp4\r\n"
    );
}

#[test]
fn gnome_payloads_from_other_file_managers_parse_with_their_operation() {
    let cases: [(&[u8], Option<ClipboardFiles>); 6] = [
        (
            b"cut\nfile:///fixture/a.txt\nfile:///fixture/b%20c.txt",
            Some(ClipboardFiles {
                sources: vec![
                    Location::local("/fixture/a.txt"),
                    Location::local("/fixture/b c.txt"),
                ],
                cut: true,
            }),
        ),
        (
            b"copy\nfile:///fixture/a.txt\n",
            Some(ClipboardFiles {
                sources: vec![Location::local("/fixture/a.txt")],
                cut: false,
            }),
        ),
        (
            b"copy\r\nfile:///fixture/a.txt\r\n",
            Some(ClipboardFiles {
                sources: vec![Location::local("/fixture/a.txt")],
                cut: false,
            }),
        ),
        (b"file:///fixture/a.txt", None),
        (b"cut", None),
        (b"", None),
    ];

    for (payload, expected) in cases {
        assert_eq!(
            parse_gnome_copied_files(payload),
            expected,
            "{:?}",
            String::from_utf8_lossy(payload)
        );
    }
}

#[test]
fn kde_cut_flag_is_a_leading_one() {
    assert!(kde_cut(b"1"));
    assert!(!kde_cut(b"0"));
    assert!(!kde_cut(b""));
}

#[test]
fn reading_honors_cut_markers_from_other_file_managers() {
    crate::test_support::gtk_test(
        "ui::browser::clipboard::formats::tests::reading_honors_cut_markers_from_other_file_managers",
        || {
            let clipboard = gdk::Display::default().expect("display").clipboard();
            let source = Location::local("/fixture/moved.txt");
            let uri_list = || {
                gdk::ContentProvider::for_bytes(
                    URI_LIST,
                    &glib::Bytes::from_static(b"file:///fixture/moved.txt\r\n"),
                )
            };
            let bytes = |mime: &str, payload: &'static [u8]| {
                gdk::ContentProvider::for_bytes(mime, &glib::Bytes::from_static(payload))
            };
            let cases = [
                (
                    "gnome cut",
                    vec![
                        bytes(GNOME_COPIED_FILES, b"cut\nfile:///fixture/moved.txt"),
                        uri_list(),
                    ],
                    true,
                ),
                (
                    "gnome copy",
                    vec![
                        bytes(GNOME_COPIED_FILES, b"copy\nfile:///fixture/moved.txt"),
                        uri_list(),
                    ],
                    false,
                ),
                (
                    "kde cut",
                    vec![uri_list(), bytes(KDE_CUT_SELECTION, b"1")],
                    true,
                ),
                (
                    "kde copy",
                    vec![uri_list(), bytes(KDE_CUT_SELECTION, b"0")],
                    false,
                ),
                ("bare uri list", vec![uri_list()], false),
                (
                    "strata cut",
                    vec![file_content(
                        &[gio::File::for_path("/fixture/moved.txt")],
                        true,
                    )],
                    true,
                ),
            ];

            for (name, providers, cut) in cases {
                let content = gdk::ContentProvider::new_union(&providers);
                clipboard.set_content(Some(&content)).expect(name);
                let read = glib::MainContext::default().block_on(read_files(&clipboard));
                assert_eq!(
                    read,
                    Some(ClipboardFiles {
                        sources: vec![source.clone()],
                        cut,
                    }),
                    "{name}"
                );
            }
        },
    );
}
