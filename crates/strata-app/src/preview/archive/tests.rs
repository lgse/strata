// SPDX-License-Identifier: MIT

use std::path::Path;

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};
use strata_fs::services::{
    ArchiveDirectory, ArchiveNode, ArchivePreviewTree, INCORRECT_ARCHIVE_PASSWORD, Preview,
    PreviewContent, PreviewEvent,
};

use super::ARCHIVE_KEY;
use crate::{
    App,
    commands::Holds,
    host::offscreen::Offscreen,
    preview::{load, viewer::Viewer},
    shell,
    slots::Part as _,
    test_support::{PATIENCE, fixture, hosted, send_key},
};

const MODULE: &str = "preview::archive::tests";

fn sample_archive_entry(folder: &Path, name: &str) -> FileEntry {
    let path = folder.join(name);
    std::fs::write(&path, b"PK\x03\x04fixture").expect("archive placeholder");
    FileEntry {
        location: Location::local(path),
        native_name: name.into(),
        thumbnail_path: None,
        display_name: name.to_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Known(128),
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        recent_uri: None,
    }
}

fn sample_tree() -> ArchivePreviewTree {
    ArchivePreviewTree {
        root: ArchiveDirectory {
            name: String::new(),
            children: vec![
                ArchiveNode::Directory(ArchiveDirectory {
                    name: "docs".to_owned(),
                    children: vec![ArchiveNode::File {
                        name: "inside.txt".to_owned(),
                        size: 6,
                    }],
                }),
                ArchiveNode::File {
                    name: "top.txt".to_owned(),
                    size: 3,
                },
            ],
        },
        file_count: 2,
    }
}

fn press_without_draining(host: &mut Offscreen, key: Key) {
    send_key(
        host,
        KeyEvent {
            key,
            mods: Mods::default(),
        },
    );
    host.runtime().frame(1.0 / 60.0);
}

fn press(host: &mut Offscreen, key: Key) {
    press_without_draining(host, key);
    while host.turn() {}
}

fn focused_key(host: &mut Offscreen) -> Option<String> {
    host.runtime()
        .elements()
        .into_iter()
        .find(|el| el.focused && !el.exiting)
        .and_then(|el| el.key)
}

fn setup_folder_with_archive(host: &mut Offscreen, name: &str) -> (tempfile::TempDir, FileEntry) {
    let folder = fixture();
    let entry = sample_archive_entry(folder.path(), name);
    host.app().open(Location::local(folder.path()));
    assert!(
        host.run_until(PATIENCE, |h| {
            h.app().browser().column_entry_counts(0).is_some()
        }),
        "folder listed"
    );
    while host.turn() {}
    host.runtime().update(|app: &mut App, _: &mut Tx| {
        shell::saved(app).change(|settings| settings.single_click_previews = false);
        let prefs = shell::preferences(app);
        app.columns.refresh_mirror_preferences(prefs);
    });
    while host.turn() {}
    let loc = entry.location.clone();
    host.runtime().update(move |app: &mut App, tx: &mut Tx| {
        let id = shell::id_of(app, &loc);
        let view = shell::shown_view(app).expect("view");
        (view.rows.select_only)(app, (0, id), tx);
        shell::publish(app);
        let key = shell::view_focus(app).expect("view").to_owned();
        tx.focus(key, true);
    });
    while host.turn() {}
    (folder, entry)
}

#[test]
fn columns_explicit_entry_focuses_archive_tree_and_left_navigates_then_returns_to_listing() {
    hosted(
        MODULE,
        "columns_explicit_entry_focuses_archive_tree_and_left_navigates_then_returns_to_listing",
        |mut host| {
            let (_folder, entry) = setup_folder_with_archive(&mut host, "bundle.zip");
            press_without_draining(&mut host, Key::Enter);

            let entry_for_ready = entry.clone();
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                let req = app.preview.loader.current_request.expect("active request");
                load::on_event(
                    app,
                    req,
                    PreviewEvent::Ready(Preview {
                        request_id: req,
                        entry: entry_for_ready,
                        content_type: "application/zip".to_owned(),
                        content: PreviewContent::Archive {
                            tree: sample_tree(),
                        },
                    }),
                    tx,
                );
            });
            while host.turn() {}

            assert_eq!(focused_key(&mut host).as_deref(), Some(ARCHIVE_KEY));
            match &host.app().preview.viewer {
                Viewer::Archive(state) => {
                    assert!(!state.passive(), "explicit entry highlights first member");
                    assert!(state.at_root());
                }
                _ => panic!("expected archive viewer"),
            }

            press(&mut host, Key::Right);
            match &host.app().preview.viewer {
                Viewer::Archive(state) => assert!(!state.at_root(), "Right opens docs/"),
                _ => panic!("expected archive viewer"),
            }
            assert_eq!(focused_key(&mut host).as_deref(), Some(ARCHIVE_KEY));

            press(&mut host, Key::Left);
            match &host.app().preview.viewer {
                Viewer::Archive(state) => assert!(state.at_root(), "Left returns to archive root"),
                _ => panic!("expected archive viewer"),
            }
            assert_eq!(focused_key(&mut host).as_deref(), Some(ARCHIVE_KEY));

            press(&mut host, Key::Left);
            assert_eq!(
                focused_key(&mut host).as_deref(),
                Some(crate::columns::VIEW),
                "Left at archive root returns focus to Columns listing"
            );
            assert!(
                host.app().preview.enabled,
                "preview stays open on root Left"
            );
        },
    );
}

fn deliver_wrong_password(host: &mut Offscreen, entry: FileEntry) {
    host.runtime().update(move |app: &mut App, tx: &mut Tx| {
        if let Viewer::ArchivePrompt(prompt) = &mut app.preview.viewer {
            prompt.edit.text = "wrong".to_owned();
        }
        app.preview.loader.focus_archive_on_ready = true;
        let req = strata_fs::services::PreviewRequestId(99);
        app.preview.loader.current_request = Some(req);
        app.preview.loader.focus_archive_request = Some(req);
        load::on_event(
            app,
            req,
            PreviewEvent::Failed {
                request_id: req,
                entry,
                message: INCORRECT_ARCHIVE_PASSWORD.to_owned(),
            },
            tx,
        );
    });
    while host.turn() {}
}

#[test]
fn password_prompt_claims_field_holds_tracks_wrong_passwords_and_dismisses_on_escape() {
    hosted(
        MODULE,
        "password_prompt_claims_field_holds_tracks_wrong_passwords_and_dismisses_on_escape",
        |mut host| {
            let (_folder, entry) = setup_folder_with_archive(&mut host, "encrypted.zip");
            press_without_draining(&mut host, Key::Right);

            let entry_needs = entry.clone();
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                let req = app.preview.loader.current_request.expect("active request");
                load::on_event(
                    app,
                    req,
                    PreviewEvent::NeedsPassword {
                        request_id: req,
                        entry: entry_needs,
                    },
                    tx,
                );
            });
            while host.turn() {}

            assert!(host.app().preview.viewer.is_prompt_focused());
            assert_eq!(focused_key(&mut host).as_deref(), Some("input"));
            assert_eq!(
                crate::preview::Preview::keyboard(host.app()),
                Some(Holds::Field),
                "focused password field reports Holds::Field so typing and Escape route properly"
            );

            deliver_wrong_password(&mut host, entry);

            match &host.app().preview.viewer {
                Viewer::ArchivePrompt(prompt) => {
                    assert!(
                        prompt.edit.text.is_empty(),
                        "wrong password clears the field"
                    );
                    assert!(prompt.error().is_some(), "wrong password records error");
                }
                _ => panic!("expected archive password prompt"),
            }

            press(&mut host, Key::Escape);
            assert!(
                !host.app().preview.enabled,
                "Escape in password field dismisses preview via Stage::PreviewPasswordEscape"
            );
            assert_eq!(
                focused_key(&mut host).as_deref(),
                Some(crate::columns::VIEW)
            );
        },
    );
}
