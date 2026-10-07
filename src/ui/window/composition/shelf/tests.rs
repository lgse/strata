// SPDX-License-Identifier: MIT

use super::{Shelf, locations_from_uri_list};
use crate::model::Location;

#[test]
fn collecting_files_preserves_sources_and_ignores_duplicate_drops()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let first_path = dir.path().join("first.txt");
    let second_path = dir.path().join("second.txt");
    std::fs::write(&first_path, "first")?;
    std::fs::write(&second_path, "second")?;
    let first = Location::local(&first_path);
    let second = Location::local(&second_path);
    let shelf = Shelf::default();

    assert_eq!(shelf.add([first.clone(), second.clone(), first.clone()]), 2);
    assert_eq!(shelf.add([second.clone()]), 0);
    assert_eq!(shelf.locations(), [first.clone(), second.clone()]);
    assert_eq!(std::fs::read_to_string(&first_path)?, "first");
    assert_eq!(std::fs::read_to_string(&second_path)?, "second");

    shelf.remove(&first);
    assert_eq!(shelf.locations(), [second]);
    shelf.clear();
    assert!(shelf.locations().is_empty());
    assert!(first_path.exists());
    assert!(second_path.exists());
    Ok(())
}

#[test]
fn uri_list_accepts_files_and_ignores_comments_and_non_uris() {
    let paths = locations_from_uri_list(
        "# Nautilus file list\r\nfile:///tmp/one.txt\r\nnot-a-uri\r\nfile:///tmp/two.txt\r\n",
    );
    assert_eq!(
        paths,
        [
            Location::local("/tmp/one.txt"),
            Location::local("/tmp/two.txt")
        ]
    );
}

#[test]
fn collecting_does_not_accept_virtual_recent_entries() {
    let shelf = Shelf::default();
    assert_eq!(shelf.add([Location::uri("recent:///item")]), 0);
    assert!(shelf.locations().is_empty());
}
