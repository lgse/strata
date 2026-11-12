// SPDX-License-Identifier: MIT

use std::{ffi::OsStr, os::unix::ffi::OsStrExt, path::Path};

use super::{Method, RevealRequest, reveal_requests};
use crate::model::Location;

fn uris(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn show_items_reveals_the_parent_directory_with_the_item_selected() {
    let requests = reveal_requests(
        Method::Items,
        &uris(&["file:///home/user/Downloads/example.md"]),
    );

    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].directory.native_path(),
        Some(Path::new("/home/user/Downloads"))
    );
    assert_eq!(
        requests[0].selection,
        vec![Location::local("/home/user/Downloads/example.md")]
    );
    assert!(!requests[0].properties);
}

#[test]
fn show_folders_opens_the_named_directories_without_a_selection() {
    let requests = reveal_requests(Method::Folders, &uris(&["file:///home/user/Downloads"]));

    assert_eq!(
        requests
            .iter()
            .map(|request| request.directory.native_path())
            .collect::<Vec<_>>(),
        vec![Some(Path::new("/home/user/Downloads"))]
    );
    assert!(requests[0].selection.is_empty());
}

#[test]
fn items_sharing_a_directory_are_selected_together_in_one_window() {
    let requests = reveal_requests(
        Method::Items,
        &uris(&[
            "file:///home/user/Downloads/first.md",
            "file:///home/user/Pictures/photo.png",
            "file:///home/user/Downloads/second.md",
        ]),
    );

    assert_eq!(
        requests
            .iter()
            .map(|request| (request.directory.native_path(), request.selection.clone()))
            .collect::<Vec<_>>(),
        vec![
            (
                Some(Path::new("/home/user/Downloads")),
                vec![
                    Location::local("/home/user/Downloads/first.md"),
                    Location::local("/home/user/Downloads/second.md"),
                ]
            ),
            (
                Some(Path::new("/home/user/Pictures")),
                vec![Location::local("/home/user/Pictures/photo.png")]
            ),
        ]
    );
}

#[test]
fn show_item_properties_marks_the_request() {
    let requests = reveal_requests(
        Method::ItemProperties,
        &uris(&["file:///home/user/Downloads/example.md"]),
    );

    assert_eq!(
        requests,
        vec![RevealRequest {
            directory: Location::local("/home/user/Downloads"),
            selection: vec![Location::local("/home/user/Downloads/example.md")],
            properties: true,
        }]
    );
}

#[test]
fn a_root_without_a_parent_is_opened_directly() {
    let requests = reveal_requests(Method::Items, &uris(&["file:///"]));

    assert_eq!(
        requests
            .iter()
            .map(|request| (request.directory.native_path(), request.selection.clone()))
            .collect::<Vec<_>>(),
        vec![(Some(Path::new("/")), Vec::new())]
    );
}

#[test]
fn a_remote_uri_reveals_its_parent_as_a_uri_location() {
    let requests = reveal_requests(Method::Items, &uris(&["sftp://host/home/user/notes.txt"]));

    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .directory
            .uri_value()
            .is_some_and(|uri| uri.ends_with("/home/user")),
        "unexpected parent location: {:?}",
        requests[0].directory
    );
    assert_eq!(
        requests[0].selection,
        vec![Location::uri("sftp://host/home/user/notes.txt")]
    );
}

#[test]
fn show_items_keeps_colliding_non_utf8_names_distinct() {
    let requests = reveal_requests(
        Method::Items,
        &uris(&[
            "file:///home/user/Downloads/bad%E8name.txt",
            "file:///home/user/Downloads/bad%E9name.txt",
        ]),
    );

    assert_eq!(requests.len(), 1);
    let downloads = Path::new("/home/user/Downloads");
    assert_eq!(
        requests[0].selection,
        [b"bad\xe8name.txt", b"bad\xe9name.txt"]
            .map(|name| Location::local(downloads.join(OsStr::from_bytes(name))))
    );
}
