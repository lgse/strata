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

fn focused_position(host: &mut Offscreen) -> Option<usize> {
    host.app().browser().focused_item().map(|(_, pos, _)| pos)
}

#[test]
fn tenxer_columns_space_marks_and_enter_activates_an_archive_without_previewing() {
    hosted(
        MODULE,
        "tenxer_columns_space_marks_and_enter_activates_an_archive_without_previewing",
        |mut host| {
            host.runtime().update(|app: &mut App, _: &mut Tx| {
                shell::saved(app).change(|settings| settings.tenxer_mode = true);
            });
            while host.turn() {}
            let (_folder, _entry) = setup_folder_with_archive(&mut host, "bundle.zip");
            let cursor = focused_position(&mut host).expect("archive focused");
            let marked = host.app().browser().selected_positions(0);

            press_without_draining(&mut host, Key::Text(" ".to_owned()));
            assert_eq!(host.app().preview.loader.current_request, None);
            assert!(!host.app().preview.enabled, "10xer Space opens no preview");
            assert_ne!(
                host.app().browser().selected_positions(0),
                marked,
                "10xer Space toggles the archive's mark"
            );
            assert_ne!(
                focused_position(&mut host),
                Some(cursor),
                "10xer Space moves the cursor down"
            );
            while host.turn() {}

            press(&mut host, Key::Up);
            assert_eq!(focused_position(&mut host), Some(cursor));
            press_without_draining(&mut host, Key::Enter);
            assert_eq!(host.app().preview.loader.current_request, None);
            assert!(!host.app().preview.enabled, "10xer Enter opens no preview");
        },
    );
}

#[test]
fn space_in_the_archive_tree_closes_the_preview_and_returns_to_the_listing() {
    hosted(
        MODULE,
        "space_in_the_archive_tree_closes_the_preview_and_returns_to_the_listing",
        |mut host| {
            let (_folder, entry) = setup_folder_with_archive(&mut host, "bundle.zip");
            press_without_draining(&mut host, Key::Right);
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                let req = app.preview.loader.current_request.expect("active request");
                load::on_event(
                    app,
                    req,
                    PreviewEvent::Ready(Preview {
                        request_id: req,
                        entry,
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

            press(&mut host, Key::Text(" ".to_owned()));
            assert!(
                !host.app().preview.enabled,
                "Space closes the preview, as on GTK"
            );
            assert_eq!(
                focused_key(&mut host).as_deref(),
                Some(crate::columns::VIEW)
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
            assert_eq!(
                host.app().preview.header.details.last().map(String::as_str),
                Some("zip"),
                "until the archive opens, the header names its extension as GTK writes it"
            );
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

/// 10xer's Shift+Tab hands the keys from the password field back to the listing and keeps
/// the prompt open (`leave_or_close_preview`, `tenxer-password-archive-keys`).
#[test]
fn tenxer_shift_tab_leaves_the_password_field_for_the_listing() {
    hosted(
        MODULE,
        "tenxer_shift_tab_leaves_the_password_field_for_the_listing",
        |mut host| {
            let (_folder, entry) = setup_folder_with_archive(&mut host, "encrypted.zip");
            host.runtime().update(|app: &mut App, _: &mut Tx| {
                shell::saved(app).change(|settings| settings.tenxer_mode = true);
            });
            while host.turn() {}
            press_without_draining(&mut host, Key::Right);
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                let req = app.preview.loader.current_request.expect("active request");
                let needs = PreviewEvent::NeedsPassword {
                    request_id: req,
                    entry,
                };
                load::on_event(app, req, needs, tx);
            });
            while host.turn() {}
            assert!(host.app().preview.viewer.is_prompt_focused());

            let shift = Mods {
                shift: true,
                ..Mods::default()
            };
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Tab,
                    mods: shift,
                },
            );
            while host.turn() {}
            assert_eq!(
                focused_key(&mut host).as_deref(),
                Some(crate::columns::VIEW)
            );
            assert!(
                matches!(host.app().preview.viewer, Viewer::ArchivePrompt(_)),
                "the prompt stays"
            );
        },
    );
}

#[test]
fn async_archive_and_prompt_focus_entry_clears_active_column_indicator() {
    hosted(
        MODULE,
        "async_archive_and_prompt_focus_entry_clears_active_column_indicator",
        |mut host| {
            let (_folder, entry) = setup_folder_with_archive(&mut host, "bundle.zip");
            let col0 = host.app().columns.list_keys()[0].clone();
            let indicator = strata_components::columns::keys::indicator(&col0);
            let has_indicator = |host: &mut Offscreen| {
                host.runtime()
                    .elements()
                    .iter()
                    .any(|el| !el.exiting && el.key.as_deref() == Some(indicator.as_str()))
            };
            assert!(
                has_indicator(&mut host),
                "active column shows its top bar before entering preview"
            );

            // Async password prompt arrival clears the active column's indicator without waiting
            // for any further user input.
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
            host.runtime().frame(0.0);
            assert!(crate::preview::owns_focus(host.app()));
            assert!(
                !has_indicator(&mut host),
                "async password prompt suppresses the active column indicator immediately"
            );

            // Async ready archive tree (when armed to focus) likewise clears the active column
            // indicator immediately.
            let entry_ready = entry.clone();
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                app.preview.owns_focus = false;
                let req = strata_fs::services::PreviewRequestId(101);
                app.preview.loader.current_request = Some(req);
                app.preview.loader.focus_archive_request = Some(req);
                load::on_event(
                    app,
                    req,
                    PreviewEvent::Ready(Preview {
                        request_id: req,
                        entry: entry_ready,
                        content_type: "application/zip".to_owned(),
                        content: PreviewContent::Archive {
                            tree: sample_tree(),
                        },
                    }),
                    tx,
                );
            });
            assert!(
                host.runtime().wake_deadline().is_some(),
                "async archive focus schedules an immediate poll wake"
            );
            host.runtime().frame(0.0);
            assert!(crate::preview::owns_focus(host.app()));
            assert!(
                !has_indicator(&mut host),
                "async archive focus suppresses the active column indicator immediately"
            );
        },
    );
}
