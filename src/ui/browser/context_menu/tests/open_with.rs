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
fn common_applications_respect_uri_capability_and_hidden_defaults() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::open_with::common_applications_respect_uri_capability_and_hidden_defaults",
        || {
            let applications = glib::user_data_dir().join("applications");
            std::fs::create_dir_all(&applications).expect("isolated applications");
            for (id, arguments, extra) in [
                ("strata-path", "%F", ""),
                ("strata-uri", "%U", "NoDisplay=true\n"),
            ] {
                std::fs::write(applications.join(format!("{id}.desktop")), format!(
                    "[Desktop Entry]\nType=Application\nName={id}\nExec=/bin/true {arguments}\nMimeType=text/plain;text/markdown;\n{extra}"
                )).expect("desktop entry");
            }
            std::fs::create_dir_all(glib::user_config_dir()).expect("isolated config");
            std::fs::write(glib::user_config_dir().join("mimeapps.list"),
                "[Default Applications]\ntext/plain=strata-path.desktop;strata-uri.desktop;\ntext/markdown=strata-path.desktop;strata-uri.desktop;\n[Added Associations]\ntext/plain=strata-path.desktop;strata-uri.desktop;\ntext/markdown=strata-path.desktop;strata-uri.desktop;\n"
            ).expect("associations");
            let types = vec!["text/plain".to_owned(), "text/markdown".to_owned()];
            let (local, default) = common_applications(&types, false);
            assert_eq!(
                default.expect("local default").id().as_deref(),
                Some("strata-path.desktop")
            );
            assert!(
                local
                    .iter()
                    .any(|app| app.id().as_deref() == Some("strata-path.desktop"))
            );
            let (remote, default) = common_applications(&types, true);
            assert_eq!(
                default.expect("URI default").id().as_deref(),
                Some("strata-uri.desktop")
            );
            assert!(remote.iter().all(|app| app.supports_uris()));
            assert_eq!(remote[0].id().as_deref(), Some("strata-uri.desktop"));
            assert!(!remote[0].should_show());
        },
    );
}

#[test]
fn prepared_selection_rejects_changed_targets() {
    let location = Location::local("/fixture/alpha.txt");
    let selection = OpenWithSelection {
        locations: vec![location.clone()],
        files: vec![gio_file_for_location(&location)],
        apps: vec![],
        default: None,
    };
    assert!(selection.entries_match_target(&[entry(location)]));
    assert!(!selection.entries_match_target(&[]));
    assert!(!selection.entries_match_target(&[entry(Location::local("/fixture/beta.txt"))]));
}

#[test]
fn preparation_preserves_uris_and_supports_folders() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::open_with::preparation_preserves_uris_and_supports_folders",
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
                    true,
                ),
                (vec![Location::local(fixture.path())], false, true),
                (vec![Location::local(&text)], true, false),
                (
                    vec![Location::local(fixture.path().join("missing"))],
                    false,
                    false,
                ),
            ] {
                let single = gtk::Button::new();
                let multiple = gtk::Button::new();
                let open = gtk::Button::new();
                open.set_visible(false);
                single.set_sensitive(false);
                multiple.set_sensitive(false);
                let result = Rc::new(RefCell::new(None));
                let generation = Rc::new(Cell::new(if stale { 2 } else { 1 }));
                prepare_open_with(
                    locations.iter().cloned().map(entry).collect(),
                    &single,
                    &multiple,
                    &open,
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
                let available = result
                    .borrow()
                    .as_ref()
                    .is_some_and(|selection| !selection.apps.is_empty());
                assert_eq!(single.is_sensitive(), available);
                assert_eq!(multiple.is_sensitive(), available);
                assert_eq!(
                    open.is_visible(),
                    result
                        .borrow()
                        .as_ref()
                        .is_some_and(|selection| selection.default.is_some())
                );
                if expected && !available {
                    assert!(multiple.tooltip_text().is_some());
                }
                if let Some(selection) = result.borrow().as_ref() {
                    for (file, location) in selection.files.iter().zip(&locations) {
                        assert!(file.equal(&gio_file_for_location(location)));
                    }
                }
            }
        },
    );
}
