// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::time::{Duration, Instant};

fn entry(location: Location) -> FileEntry {
    FileEntry {
        location,
        native_name: "fixture".into(),
        thumbnail_path: None,
        display_name: "fixture".into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
    }
}

#[test]
fn prepared_selection_rejects_changed_targets() {
    let location = Location::local("/fixture/alpha.txt");
    let selection = OpenWithSelection {
        locations: vec![location.clone()],
        files: vec![gio_file_for_location(&location)],
        apps: vec![],
    };
    assert!(selection.entries_match_target(&[entry(location)]));
    assert!(!selection.entries_match_target(&[]));
    assert!(!selection.entries_match_target(&[entry(Location::local("/fixture/beta.txt"))]));
}

#[test]
fn preparation_preserves_file_uris_and_rejects_incompatible_or_stale_queries() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::open_with::preparation_preserves_file_uris_and_rejects_incompatible_or_stale_queries",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let text = fixture.path().join("alpha.txt");
            let other = fixture.path().join("beta.txt");
            let image = fixture.path().join("image.png");
            std::fs::write(&text, "alpha\n").expect("text file");
            std::fs::write(&other, "beta\n").expect("second text file");
            std::fs::write(&image, b"\x89PNG\r\n\x1a\n").expect("image");
            let text_file = gio::File::for_path(&text);
            for (locations, stale, expected) in [
                (
                    vec![Location::local(&text), Location::local(&other)],
                    false,
                    true,
                ),
                (vec![Location::uri(text_file.uri().as_str())], false, true),
                (
                    vec![Location::local(&text), Location::local(&image)],
                    false,
                    false,
                ),
                (vec![Location::local(fixture.path())], false, false),
                (vec![Location::local(&text)], true, false),
                (
                    vec![Location::local(fixture.path().join("missing"))],
                    false,
                    false,
                ),
            ] {
                let single = gtk::Button::new();
                let multiple = gtk::Button::new();
                single.set_sensitive(false);
                multiple.set_sensitive(false);
                let result = Rc::new(RefCell::new(None));
                let generation = Rc::new(Cell::new(if stale { 2 } else { 1 }));
                prepare_open_with(
                    locations.iter().cloned().map(entry).collect(),
                    &single,
                    &multiple,
                    &result,
                    &generation,
                    1,
                );
                let deadline = Instant::now() + Duration::from_millis(250);
                while Instant::now() < deadline {
                    glib::MainContext::default().iteration(false);
                    std::thread::sleep(Duration::from_millis(1));
                }
                assert_eq!(result.borrow().is_some(), expected, "{locations:?}");
                assert_eq!(single.is_sensitive(), expected);
                assert_eq!(multiple.is_sensitive(), expected);
                if let Some(selection) = result.borrow().as_ref() {
                    for (file, location) in selection.files.iter().zip(&locations) {
                        assert!(file.equal(&gio_file_for_location(location)));
                    }
                }
            }
        },
    );
}
