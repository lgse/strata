// SPDX-License-Identifier: MIT

use std::{collections::HashSet, path::PathBuf};

use rune::{Key, KeyEvent, Mods};
use serde_json::Value;
use strata_core::model::Location;
use strata_fs::services::DeleteBlocker;

use super::{DeleteErrorRetry, Failed, completed_with_errors, show_partial_operation_failure};
use crate::{
    App, error_dialog,
    host::offscreen::Offscreen,
    test_support::{PATIENCE, hosted, send_key},
};

const MODULE: &str = "trash::delete_error::tests";

fn collect_role<'a>(node: &'a Value, role: &str, out: &mut Vec<&'a Value>) {
    for child in node["children"].as_array().map_or(&[][..], Vec::as_slice) {
        if child["role"] == role {
            out.push(child);
        }
        collect_role(child, role, out);
    }
}

fn dialog_buttons(host: &Offscreen) -> Option<(String, Vec<String>, Option<String>)> {
    let tree = host.tree();
    let mut dialogs = Vec::new();
    collect_role(&tree, "dialog", &mut dialogs);
    let dialog = dialogs.first()?;
    let mut buttons = Vec::new();
    collect_role(dialog, "button", &mut buttons);
    collect_role(dialog, "default-button", &mut buttons);
    let mut labels = Vec::new();
    let mut focused = None;
    for button in buttons {
        let name = button["name"].as_str().unwrap_or_default().to_owned();
        if button["states"]
            .as_array()
            .is_some_and(|states| states.iter().any(|state| state == "focused"))
        {
            focused = Some(name.clone());
        }
        labels.push(name);
    }
    Some((
        dialog["name"].as_str().unwrap_or_default().to_owned(),
        labels,
        focused,
    ))
}

fn wait_dialog(host: &mut Offscreen, expected: &str) -> (Vec<String>, Option<String>) {
    let shown = host.run_until(PATIENCE, |host| {
        dialog_buttons(host).is_some_and(|(title, _, _)| title == expected)
    });
    assert!(
        shown,
        "expected dialog {expected:?}, got {:?}",
        dialog_buttons(host)
    );
    let (_, labels, focused) = dialog_buttons(host).expect("dialog");
    (labels, focused)
}

fn wait_no_dialog(host: &mut Offscreen) {
    let gone = host.run_until(PATIENCE, |host| dialog_buttons(host).is_none());
    assert!(gone, "dialog remained open: {:?}", dialog_buttons(host));
}

fn click_button(host: &mut Offscreen, label: &str) {
    while host.turn() {
        host.runtime().settle_all();
    }
    let tree = host.tree();
    let mut dialogs = Vec::new();
    collect_role(&tree, "dialog", &mut dialogs);
    let dialog = dialogs.first().expect("open dialog");
    let mut buttons = Vec::new();
    collect_role(dialog, "button", &mut buttons);
    collect_role(dialog, "default-button", &mut buttons);
    let key = buttons
        .iter()
        .find(|button| button["name"] == label)
        .and_then(|button| button["key"].as_str())
        .unwrap_or_else(|| panic!("no dialog button named {label:?}"))
        .to_owned();
    let elements = host.runtime().elements();
    let drawn = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key.as_str()))
        .unwrap_or_else(|| panic!("button {label:?} ({key}) is not drawn"));
    let [x, y, w, h] = drawn.visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    while host.turn() {}
}

#[test]
fn show_in_folder_names_one_of_several_blockers_and_selects_it() {
    hosted(
        MODULE,
        "show_in_folder_names_one_of_several_blockers_and_selects_it",
        |mut host| {
            let fixture = tempfile::tempdir().expect("fixture");
            let root = fixture.path().join("to-delete");
            let nested = root.join("nested");
            std::fs::create_dir_all(&nested).expect("folder");
            std::fs::write(nested.join("blocked.txt"), b"keep").expect("blocker");

            for (one_of_several, label) in [
                (false, "Show in Folder"),
                (true, "Show “blocked.txt” in Folder"),
            ] {
                let start = Location::local(fixture.path());
                host.app().open(start.clone());
                assert!(
                    host.run_until(PATIENCE, |host| {
                        host.app().browser().location_at(0).as_ref() == Some(&start)
                            && host.app().browser().column_entry_counts(0).is_some()
                    }),
                    "initial folder listed"
                );
                let blocker = DeleteBlocker {
                    root: root.clone(),
                    child: PathBuf::from("nested/blocked.txt"),
                    one_of_several,
                };
                host.runtime().update(move |app: &mut App, _| {
                    show_partial_operation_failure(
                        app,
                        "to-delete/nested/blocked.txt: Permission denied".into(),
                        Some(blocker),
                        DeleteErrorRetry {
                            entries: Vec::new(),
                            require_choice: false,
                        },
                    );
                });
                let (buttons, focused) = wait_dialog(&mut host, "Completed with errors");
                assert!(buttons.iter().any(|name| name == "Close"), "{buttons:?}");
                assert!(buttons.iter().any(|name| name == label), "{buttons:?}");
                assert!(
                    !buttons.iter().any(|name| name == "Delete Permanently"),
                    "permission refusal must not offer permanent delete: {buttons:?}"
                );
                assert_eq!(focused.as_deref(), Some("Close"));

                click_button(&mut host, label);
                wait_no_dialog(&mut host);

                let expected_item = Location::local(nested.join("blocked.txt"));
                let expected_dir = Location::local(&nested);
                let revealed = host.run_until(PATIENCE, |host| {
                    host.app().browser().active_location().as_ref() == Some(&expected_dir)
                        && host
                            .app()
                            .browser()
                            .focused_item()
                            .is_some_and(|(_, _, entry)| entry.location == expected_item)
                });
                assert!(revealed, "blocker should be revealed and selected");
            }
        },
    );
}

#[test]
fn unverifiable_blocker_shows_unable_to_open_location() {
    hosted(
        MODULE,
        "unverifiable_blocker_shows_unable_to_open_location",
        |mut host| {
            let fixture = tempfile::tempdir().expect("fixture");
            let root = fixture.path().join("to-delete");
            std::fs::create_dir_all(&root).expect("folder");
            host.app().open(Location::local(fixture.path()));
            assert!(host.run_until(PATIENCE, |host| {
                host.app().browser().column_entry_counts(0).is_some()
            }));

            let blocker = DeleteBlocker {
                root,
                child: PathBuf::from("gone/missing.txt"),
                one_of_several: false,
            };
            host.runtime().update(move |app: &mut App, _| {
                show_partial_operation_failure(
                    app,
                    "to-delete/gone/missing.txt: Permission denied".into(),
                    Some(blocker),
                    DeleteErrorRetry {
                        entries: Vec::new(),
                        require_choice: false,
                    },
                );
            });
            wait_dialog(&mut host, "Completed with errors");
            click_button(&mut host, "Show in Folder");

            wait_dialog(&mut host, "Unable to open location");
            let details = error_dialog::shown_details(host.app()).expect("error details");
            assert_eq!(details.4, "The item's location could not be verified.");
        },
    );
}

#[test]
fn mixed_failures_offer_done_reveal_and_delete_permanently() {
    hosted(
        MODULE,
        "mixed_failures_offer_done_reveal_and_delete_permanently",
        |mut host| {
            let fixture = tempfile::tempdir().expect("fixture");
            let retry_file = fixture.path().join("retryable.txt");
            let root = fixture.path().join("blocked-dir");
            std::fs::write(&retry_file, b"retry").expect("retryable");
            std::fs::create_dir_all(&root).expect("blocked dir");
            std::fs::write(root.join("locked.txt"), b"locked").expect("locked");

            host.app().open(Location::local(fixture.path()));
            assert!(host.run_until(PATIENCE, |host| {
                host.app()
                    .browser()
                    .column_entry_counts(0)
                    .is_some_and(|counts| counts.total >= 2)
            }));

            let retry_location = Location::local(&retry_file);
            let blocker = DeleteBlocker {
                root,
                child: PathBuf::from("locked.txt"),
                one_of_several: true,
            };
            host.runtime().update(move |app: &mut App, _| {
                let names = HashSet::from(["retryable.txt".to_owned()]);
                app.trash.trashing = app.browser.entries_named(&names);
                completed_with_errors(
                    app,
                    Failed {
                        message: "blocked-dir/locked.txt: Permission denied\n\n• retryable.txt: This location doesn't support Trash.".into(),
                        retryable: vec![retry_location],
                        others_failed: true,
                        blocker: Some(blocker),
                    },
                );
            });

            let (buttons, focused) = wait_dialog(&mut host, "Completed with errors");
            assert!(buttons.iter().any(|name| name == "Done"), "{buttons:?}");
            assert!(
                buttons
                    .iter()
                    .any(|name| name == "Show “locked.txt” in Folder"),
                "{buttons:?}"
            );
            assert!(
                buttons.iter().any(|name| name == "Delete Permanently"),
                "{buttons:?}"
            );
            assert_eq!(focused.as_deref(), Some("Delete Permanently"));

            click_button(&mut host, "Delete Permanently");
            let (_, focused) = wait_dialog(&mut host, "Permanently delete 1 item?");
            assert_eq!(focused.as_deref(), Some("Cancel"));

            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Escape,
                    mods: Mods::default(),
                },
            );
            wait_no_dialog(&mut host);
            assert!(retry_file.exists(), "Escape should keep the retryable file");
        },
    );
}
