// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::model::Location;
use gtk::gio;
use std::path::Path;

#[test]
fn incoming_file_lists_preserve_local_and_remote_locations() {
    let files = gtk::gdk::FileList::from_array(&[
        gio::File::for_path("/fixture/photo.raw"),
        gio::File::for_uri("sftp://host.example/home/user/video.mp4"),
    ]);
    let value = files.to_value();

    assert_eq!(
        locations_from_file_list_value(&value),
        Some(vec![
            Location::local("/fixture/photo.raw"),
            Location::uri("sftp://host.example/home/user/video.mp4"),
        ])
    );
}

#[test]
fn incoming_file_lists_sanitize_remote_credentials() {
    let files = gtk::gdk::FileList::from_array(&[
        gio::File::for_uri("smb://user%3Asecret@host/share"),
        gio::File::for_uri("smb://user%3Bpassword=secret@host/share"),
        gio::File::for_uri("sftp://user@host/home/user/video.mp4"),
    ]);

    let locations = locations_from_file_list_value(&files.to_value())
        .expect("file list should contain sanitized locations");
    let uris = locations
        .iter()
        .map(|location| {
            location
                .uri_value()
                .expect("remote location should have a URI")
                .trim_end_matches('/')
        })
        .collect::<Vec<_>>();
    assert_eq!(
        uris,
        [
            "smb://user@host/share",
            "smb://user@host/share",
            "sftp://user@host/home/user/video.mp4",
        ]
    );
}

#[test]
fn local_file_drops_prefer_move_while_external_drops_prefer_copy() {
    let both = gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE;

    assert_eq!(
        preferred_file_drop_action(both, true),
        gtk::gdk::DragAction::MOVE
    );
    assert_eq!(
        preferred_file_drop_action(both, false),
        gtk::gdk::DragAction::COPY
    );
    assert_eq!(
        preferred_file_drop_action(gtk::gdk::DragAction::MOVE, false),
        gtk::gdk::DragAction::MOVE
    );
}

#[test]
fn cut_clipboard_locations_match_regardless_of_order() {
    let first = Location::local("/fixture/first");
    let second = Location::local("/fixture/second");

    assert!(same_locations(
        &[first.clone(), second.clone()],
        &[second, first]
    ));
    assert!(!same_locations(&[], &[]));
    assert!(!same_locations(
        &[Location::local("/fixture/first")],
        &[Location::local("/fixture/other")]
    ));
    assert!(!same_locations(
        &[
            Location::local("/fixture/first"),
            Location::local("/fixture/first")
        ],
        &[
            Location::local("/fixture/first"),
            Location::local("/fixture/second")
        ]
    ));
}

#[test]
fn cut_matches_gio_equivalent_representations() {
    let native = Location::local("/fixture/first");
    let uri = Location::uri("file:///fixture/first");

    assert!(locations_equal(&native, &uri));
    assert!(same_locations(
        std::slice::from_ref(&native),
        std::slice::from_ref(&uri)
    ));
    assert!(!same_locations(
        std::slice::from_ref(&native),
        std::slice::from_ref(&Location::uri("file:///fixture/other"))
    ));
}

#[test]
fn cleared_shared_cut_is_not_revived_by_stale_view_state() {
    let native = Location::local("/fixture/first");
    let uri = Location::uri("file:///fixture/first");

    set_shared_cut(std::slice::from_ref(&native));
    assert!(is_cut_match(std::slice::from_ref(&uri)));

    clear_shared_cut();
    assert!(!is_cut_match(std::slice::from_ref(&native)));
}

#[test]
fn completed_moves_match_gio_equivalent_cut_entries() {
    let mut cut = vec![Location::local("/fixture/first")];

    retain_untransferred(&mut cut, &[Location::uri("file:///fixture/first")]);

    assert!(cut.is_empty());
}

#[test]
fn completed_moves_are_removed_from_the_cut_list() {
    let first = Location::local("/fixture/first");
    let second = Location::local("/fixture/second");
    let third = Location::local("/fixture/third");
    let mut cut = vec![first.clone(), second.clone(), third.clone()];

    retain_untransferred(&mut cut, &[first, third]);

    assert_eq!(cut, vec![second]);
}

#[test]
fn shell_escape_path_escapes_spaces_and_metacharacters_but_not_filename_unicode() {
    assert_eq!(
        shell_escape_path(Path::new("/mnt/Mass 1/Movies\u{2044}TV")),
        "/mnt/Mass\\ 1/Movies\u{2044}TV"
    );
    assert_eq!(
        shell_escape_path(Path::new("/tmp/archive (final) v1.2.tar.gz")),
        "/tmp/archive\\ \\(final\\)\\ v1.2.tar.gz"
    );
    assert_eq!(
        shell_escape_path(Path::new("/home/user/plain")),
        "/home/user/plain"
    );
}

#[test]
fn copy_path_text_adds_trailing_slash_to_directories_only() {
    let directory = Location::local("/mnt/Mass 1/Movies\u{2044}TV");
    assert_eq!(
        copy_path_text(&directory, true),
        "/mnt/Mass\\ 1/Movies\u{2044}TV/"
    );
    assert_eq!(
        copy_path_text(&directory, false),
        "/mnt/Mass\\ 1/Movies\u{2044}TV"
    );
}

#[test]
fn copy_path_text_keeps_uris_unescaped() {
    let remote = Location::uri("smb://server/share/folder");
    assert_eq!(copy_path_text(&remote, true), "smb://server/share/folder");
}

#[test]
fn shell_escape_path_preserves_newlines_and_single_quotes() {
    assert_eq!(
        shell_escape_path(Path::new("/tmp/line\nbob's notes")),
        "'/tmp/line\nbob'\\''s notes'"
    );
}
