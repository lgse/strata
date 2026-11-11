// SPDX-License-Identifier: MIT

use super::{Naming, name_field_error, rename_stem_end};

const BLANK: Option<&str> = Some("Enter a name");
const SLASH: Option<&str> = Some("Names cannot contain /");
const RESERVED: Option<&str> = Some("That name is reserved");
const FOLDER: Option<&str> = Some("An existing file can’t become a folder");

#[test]
fn a_rename_selects_up_to_the_last_dot_that_does_not_lead_the_name() {
    for (name, end) in [
        ("report.txt", 6),
        ("archive.tar.gz", 11),
        ("README", 6),
        (".gitignore", 10),
        (".config.toml", 7),
        ("trailing.", 8),
        ("", 0),
        // Characters, not bytes: a text field counts its selection in them.
        ("résumé.pdf", 6),
        ("日本語.txt", 3),
        ("māo", 3),
    ] {
        assert_eq!(rename_stem_end(name), end, "{name:?}");
    }
}

#[test]
fn an_empty_field_is_never_an_error() {
    for naming in [Naming::Single, Naming::Path, Naming::NewItem] {
        for is_directory in [false, true] {
            let error = name_field_error("", naming, is_directory);
            assert_eq!(error, None, "{naming:?}, a folder: {is_directory}");
        }
    }
}

#[test]
fn a_typed_name_is_refused_by_the_rule_its_naming_asks() {
    for (name, naming, is_directory, error) in [
        ("notes.txt", Naming::Single, false, None),
        ("docs/notes.txt", Naming::Single, false, SLASH),
        ("..", Naming::Single, false, RESERVED),
        ("   ", Naming::Single, false, BLANK),
        // In a local folder a name with a slash is a path below it.
        ("docs/notes.txt", Naming::Path, false, None),
        ("../notes.txt", Naming::Path, false, None),
        ("bad//name", Naming::Path, false, BLANK),
        ("docs/..", Naming::Path, false, RESERVED),
        // A trailing slash names a folder, which only a folder or a new item can become.
        ("docs/", Naming::Path, true, None),
        ("docs/", Naming::Path, false, FOLDER),
        ("docs/", Naming::NewItem, false, None),
        ("a//b/", Naming::NewItem, false, BLANK),
    ] {
        assert_eq!(
            name_field_error(name, naming, is_directory),
            error,
            "{name:?} as {naming:?}, a folder: {is_directory}"
        );
    }
}
