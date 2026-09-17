// SPDX-License-Identifier: MIT

use std::ffi::OsString;

use super::*;
use crate::model::{Location, MetadataValue};

fn test_entry(name: &str, kind: EntryKind) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/tmp/{name}")),
        thumbnail_path: None,
        native_name: OsString::from(name),
        display_name: name.to_owned(),
        kind,
        size: MetadataValue::Known(100),
        modified_unix_seconds: MetadataValue::Known(1000),
        mode: MetadataValue::Known(0o644),
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
    }
}

#[test]
fn entry_type_identifies_directories_and_broken_links() {
    let dir = test_entry("documents", EntryKind::Directory);
    assert_eq!(entry_type(&dir), EntryType::Folder);
    assert_eq!(entry_type_description(&dir), "Folder");

    let dir_link = test_entry("docs_link", EntryKind::DirectorySymbolicLink);
    assert_eq!(entry_type(&dir_link), EntryType::Folder);
    assert_eq!(entry_type_description(&dir_link), "Folder");

    let broken = test_entry("broken", EntryKind::SymbolicLink);
    assert_eq!(entry_type(&broken), EntryType::BrokenLink);
    assert_eq!(entry_type_description(&broken), "Broken link");

    let other = test_entry("socket", EntryKind::Other);
    assert_eq!(entry_type(&other), EntryType::Other);
    assert_eq!(entry_type_description(&other), "Other");
}

#[test]
fn entry_type_identifies_known_and_unknown_files() {
    let json_file = test_entry("package.json", EntryKind::File);
    let expected_json = gio::content_type_get_description(
        &gio::content_type_guess(Some(Path::new("package.json")), None::<&[u8]>).0,
    );
    assert_eq!(
        entry_type(&json_file),
        EntryType::Known(expected_json.clone().into())
    );
    assert_eq!(entry_type_description(&json_file), expected_json);

    let unknown_file = test_entry("blob.qqqqq", EntryKind::File);
    assert_eq!(entry_type(&unknown_file), EntryType::Other);
    assert_eq!(entry_type_description(&unknown_file), "Other");
}

#[test]
fn mime_description_for_name_uses_shared_database() {
    let json_desc = mime_description_for_name("data.json");
    assert_ne!(json_desc, "Other");
    assert_ne!(json_desc, "File");

    let unknown_desc = mime_description_for_name("archive.xyzunknown");
    assert_eq!(unknown_desc, "Other");
}

#[test]
fn filenames_with_the_same_simple_suffix_agree() {
    let first = mime_description_for_name("alpha.py");
    let second = mime_description_for_name("beta.py");
    assert_eq!(first, second);
    assert_ne!(first, "Other");
}

#[test]
fn cached_guesses_preserve_compound_suffixes_and_filename_globs() {
    let names = ["file.gz", "file.tar.gz", "file.am", "Makefile.am"];
    for reverse in [false, true] {
        TYPE_CACHE.with_borrow_mut(HashMap::clear);
        let mut names = names;
        if reverse {
            names.reverse();
        }
        for name in names {
            assert_eq!(
                mime_description_for_name(name),
                guess_mime_description(name),
                "{name}"
            );
        }
    }
}
