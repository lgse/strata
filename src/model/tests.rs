// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn breadcrumbs_preserve_each_native_ancestor() {
    let location = Location::local("/home/user/project");

    assert_eq!(
        location.breadcrumbs(),
        vec![
            Location::local("/"),
            Location::local("/home"),
            Location::local("/home/user"),
            Location::local("/home/user/project"),
        ]
    );
}

#[test]
fn uri_locations_remain_explicit_and_have_one_breadcrumb() {
    let trash = Location::uri("trash:///");

    assert_eq!(trash.native_path(), None);
    assert_eq!(trash.uri_value(), Some("trash:///"));
    assert_eq!(trash.display_name(), "Trash");
    assert_eq!(trash.display_path(), "trash:///");
    assert_eq!(trash.breadcrumbs(), vec![trash.clone()]);
    assert_eq!(trash.parent(), None);
}

#[test]
fn backend_names_distinguish_native_remote_and_malformed_locations() {
    assert_eq!(
        Location::local("/home/alice/private").backend_name(),
        "native"
    );
    assert_eq!(
        Location::uri("sftp://example.com/private").backend_name(),
        "sftp"
    );
    assert_eq!(Location::uri("not a uri").backend_name(), "uri");
}

#[test]
fn diagnostic_paths_preserve_native_paths_and_redact_remote_secrets() {
    assert_eq!(
        Location::local("/home/alice/private").diagnostic_path(),
        "/home/alice/private"
    );
    assert_eq!(
        Location::uri(
            "sftp://alice:password;key=secret@example.com/home/alice?token=secret#private-fragment"
        )
        .diagnostic_path(),
        "sftp://example.com/home/alice"
    );
    assert_eq!(
        Location::uri("not a uri with alice:password@example.com").diagnostic_path(),
        "<invalid-uri>"
    );
}

#[test]
fn root_has_one_breadcrumb() {
    assert_eq!(
        Location::local("/").breadcrumbs(),
        vec![Location::local("/")]
    );
}
