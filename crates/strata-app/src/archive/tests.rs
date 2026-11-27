// SPDX-License-Identifier: MIT

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    rc::Rc,
};

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};
use strata_fs::services::{CancelledOperation, OperationEvent};

use super::{Archive, extract_entry, extract_to_chosen, on_background_completed};
use crate::{
    App,
    commands::{self, Holds, On, Outcome},
    host::offscreen::Offscreen,
    shell,
    slots::Part,
    test_support::{PATIENCE, fixture, held::HeldExtractions, hosted, send_key, view::idle},
};

const MODULE: &str = "archive::tests";

fn repo_fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

fn type_text(host: &mut Offscreen, text: &str) {
    press(host, Key::Text(text.to_owned()), Mods::default());
}

fn open_fixture(host: &mut Offscreen, root: &Path) {
    host.app().open(Location::local(root));
    let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, listed), "the folder is listed");
    while host.turn() {}
}

fn run_row_command(host: &mut Offscreen, command: &str, path: &Path) {
    let id = shell::id_of(host.app(), &Location::local(path));
    let command = command.to_owned();
    host.runtime().update(move |app: &mut App, tx: &mut Tx| {
        assert_eq!(
            commands::run(app, &command, On::Row((0, id)), tx),
            Outcome::Ran
        );
    });
    idle(host);
}

fn has_a11y_node(node: &serde_json::Value, role: &str, name: &str) -> bool {
    let matches_self = node.get("role").and_then(|v| v.as_str()) == Some(role)
        && node.get("name").and_then(|v| v.as_str()) == Some(name);
    matches_self
        || node
            .get("children")
            .and_then(|v| v.as_array())
            .is_some_and(|children| {
                children
                    .iter()
                    .any(|child| has_a11y_node(child, role, name))
            })
}

fn mentions(node: &serde_json::Value, needle: &str) -> bool {
    match node {
        serde_json::Value::String(text) => text.contains(needle),
        serde_json::Value::Array(nodes) => nodes.iter().any(|node| mentions(node, needle)),
        serde_json::Value::Object(fields) => fields.values().any(|node| mentions(node, needle)),
        _ => false,
    }
}

fn select_names(host: &mut Offscreen, names: &[&str]) {
    let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    assert!(
        host.app().browser().select_entries_by_name_at(0, &names),
        "{names:?} are listed"
    );
    while host.turn() {}
}

fn selected_names(host: &Offscreen) -> Vec<String> {
    let selected = host.app().browser().selected_entries();
    selected
        .into_iter()
        .map(|entry| entry.display_name)
        .collect()
}

fn dummy_archive_at(path: &Path, name: &str) -> FileEntry {
    FileEntry {
        location: Location::local(path),
        thumbnail_path: None,
        native_name: name.into(),
        display_name: name.to_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        mode: MetadataValue::Known(0o644),
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
    }
}

fn write_single_file_tar(archive: &Path, member: &str, contents: &[u8]) {
    let mut header = [0u8; 512];
    header[..member.len()].copy_from_slice(member.as_bytes());
    header[100..107].copy_from_slice(b"0000644");
    header[108..115].copy_from_slice(b"0000000");
    header[116..123].copy_from_slice(b"0000000");
    let size_octal = format!("{:011o}", contents.len());
    header[124..135].copy_from_slice(size_octal.as_bytes());
    header[136..147].copy_from_slice(b"14000000000");
    header[148..156].fill(b' ');
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let checksum: u32 = header.iter().map(|&b| u32::from(b)).sum();
    let checksum_octal = format!("{checksum:06o}\0 ");
    header[148..156].copy_from_slice(checksum_octal.as_bytes());

    let mut bytes = Vec::from(header);
    bytes.extend_from_slice(contents);
    let pad = (512 - (contents.len() % 512)) % 512;
    bytes.resize(bytes.len() + pad + 1024, 0);
    std::fs::write(archive, bytes).expect("write tar fixture");
}

#[test]
fn rootless_archive_shows_cannot_extract_and_extract_command_unpacks_and_selects() {
    hosted(
        MODULE,
        "rootless_archive_shows_cannot_extract_and_extract_command_unpacks_and_selects",
        |mut host| {
            let folder = fixture();
            let tar = folder.path().join("sample.tar");
            write_single_file_tar(&tar, "unpacked.txt", b"from tar\n");
            open_fixture(&mut host, folder.path());

            // An archive path with no parent directory opens "Cannot extract".
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                extract_entry(app, dummy_archive_at(Path::new("/"), "root.zip"), tx);
            });
            idle(&mut host);
            assert!(has_a11y_node(&host.tree(), "dialog", "Cannot extract"));
            press(&mut host, Key::Escape, Mods::default());

            // Running "extract" on sample.tar unpacks unpacked.txt and selects it in place.
            run_row_command(&mut host, "extract", &tar);
            let extracted = folder.path().join("unpacked.txt");
            let selected = |h: &Offscreen| {
                extracted.exists()
                    && h.app()
                        .browser()
                        .selected_entries()
                        .iter()
                        .any(|entry| entry.display_name == "unpacked.txt")
            };
            assert!(
                host.run_until(PATIENCE, selected),
                "unpacked.txt is extracted and selected"
            );
        },
    );
}

#[test]
fn encrypted_archive_prompts_validates_empty_reopens_on_wrong_password_and_extracts() {
    hosted(
        MODULE,
        "encrypted_archive_prompts_validates_empty_reopens_on_wrong_password_and_extracts",
        |mut host| {
            let folder = fixture();
            let encrypted = folder.path().join("content-encrypted.7z");
            std::fs::copy(
                repo_fixture("tests/qa/fixtures/content-encrypted.7z"),
                &encrypted,
            )
            .expect("seed encrypted 7z");
            open_fixture(&mut host, folder.path());

            run_row_command(&mut host, "extract", &encrypted);
            assert!(
                host.run_until(PATIENCE, |h| h.app().archive.open),
                "Extract password dialog opens"
            );
            assert_eq!(
                <Archive as Part>::keyboard(host.app()),
                Some(Holds::Modal { field: true })
            );

            // Empty submit keeps the dialog open and shows "Enter a password".
            press(&mut host, Key::Enter, Mods::default());
            assert!(host.app().archive.open);
            assert!(has_a11y_node(&host.tree(), "label", "Enter a password"));

            // Wrong password reopens Extract with "Invalid password" and no generic failure dialog.
            type_text(&mut host, "wrong");
            assert!(!has_a11y_node(&host.tree(), "label", "Enter a password"));
            press(&mut host, Key::Enter, Mods::default());
            let reopened = |h: &Offscreen| {
                h.app().archive.open
                    && h.app()
                        .archive
                        .form
                        .as_ref()
                        .is_some_and(|form| form.error == Some("Invalid password"))
            };
            assert!(
                host.run_until(PATIENCE, reopened),
                "wrong password reopens Extract with Invalid password"
            );
            assert!(has_a11y_node(&host.tree(), "label", "Invalid password"));
            assert!(!has_a11y_node(
                &host.tree(),
                "dialog",
                "Unable to complete operation"
            ));
            assert!(!folder.path().join("protected.txt").exists());

            // Correct password extracts protected.txt.
            type_text(&mut host, "secret");
            press(&mut host, Key::Enter, Mods::default());
            let extracted = folder.path().join("protected.txt");
            assert!(
                host.run_until(PATIENCE, |_| extracted.exists()),
                "correct password extracts protected.txt"
            );
            assert_eq!(
                std::fs::read_to_string(&extracted).expect("extracted contents"),
                "password retry works\n"
            );
        },
    );
}

#[test]
fn abandoned_extract_to_clears_navigation_before_subsequent_extract_here() {
    hosted(
        MODULE,
        "abandoned_extract_to_clears_navigation_before_subsequent_extract_here",
        |mut host| {
            let folder = fixture();
            let encrypted = folder.path().join("content-encrypted.7z");
            let valid = folder.path().join("later.tar");
            std::fs::copy(
                repo_fixture("tests/qa/fixtures/content-encrypted.7z"),
                &encrypted,
            )
            .expect("seed encrypted 7z");
            write_single_file_tar(&valid, "later.txt", b"later extraction\n");
            open_fixture(&mut host, folder.path());

            let documents = Location::local(folder.path().join("documents"));
            let entry = dummy_archive_at(&encrypted, "content-encrypted.7z");
            let chosen = documents.clone();
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                extract_to_chosen(app, entry, chosen, tx);
            });
            assert!(
                host.run_until(PATIENCE, |h| h.app().archive.open),
                "Extract password dialog opens for Extract to"
            );
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().archive.open);

            // Subsequent Extract here stays in the root folder instead of navigating to documents.
            run_row_command(&mut host, "extract", &valid);
            let extracted = folder.path().join("later.txt");
            assert!(
                host.run_until(PATIENCE, |_| extracted.exists()),
                "later Extract here completes"
            );
            assert_eq!(
                host.app().browser().active_location(),
                Some(Location::local(folder.path()))
            );
            assert!(!folder.path().join("documents/later.txt").exists());
        },
    );
}

#[test]
fn compression_collision_supports_keep_both_and_replace() {
    hosted(
        MODULE,
        "compression_collision_supports_keep_both_and_replace",
        |mut host| {
            let folder = fixture();
            let original = folder.path().join("readme.md.zip");
            std::fs::write(&original, "original archive").expect("seed existing archive");
            open_fixture(&mut host, folder.path());

            // Keep Both creates readme.md (1).zip without touching readme.md.zip.
            run_row_command(&mut host, "compress", &folder.path().join("readme.md"));
            press(&mut host, Key::Enter, Mods::default());
            assert!(
                host.run_until(PATIENCE, |h| crate::conflict_dialog::is_open(h.app())),
                "collision prompt opens"
            );
            press(
                &mut host,
                Key::Tab,
                Mods {
                    shift: true,
                    ..Mods::default()
                },
            );
            press(&mut host, Key::Enter, Mods::default());
            let numbered = folder.path().join("readme.md (1).zip");
            assert!(
                host.run_until(PATIENCE, |_| numbered.exists()),
                "Keep Both writes readme.md (1).zip"
            );
            assert_eq!(
                std::fs::read_to_string(&original).expect("original intact"),
                "original archive"
            );

            // Replace overwrites readme.md.zip with a valid zip archive.
            run_row_command(&mut host, "compress", &folder.path().join("readme.md"));
            press(&mut host, Key::Enter, Mods::default());
            assert!(
                host.run_until(PATIENCE, |h| crate::conflict_dialog::is_open(h.app())),
                "second collision prompt opens"
            );
            press(&mut host, Key::Enter, Mods::default());
            let replaced = |_: &Offscreen| {
                std::fs::read(&original)
                    .map(|bytes| bytes.starts_with(b"PK\x03\x04"))
                    .unwrap_or(false)
            };
            assert!(
                host.run_until(PATIENCE, replaced),
                "Replace overwrites readme.md.zip"
            );
        },
    );
}

fn is_selected_at(host: &Offscreen, expected_dir: &Location, name: &str) -> bool {
    host.app().browser().active_location().as_ref() == Some(expected_dir)
        && host
            .app()
            .browser()
            .selected_entries()
            .iter()
            .any(|entry| entry.display_name == name)
}

#[test]
fn extract_to_chosen_selects_result_in_same_or_different_folder_across_reset() {
    hosted(
        MODULE,
        "extract_to_chosen_selects_result_in_same_or_different_folder_across_reset",
        |mut host| {
            let folder = fixture();
            let same_tar = folder.path().join("same.tar");
            let other_tar = folder.path().join("other.tar");
            write_single_file_tar(&same_tar, "same_dir.txt", b"in root\n");
            write_single_file_tar(&other_tar, "into_docs.txt", b"in documents\n");
            std::fs::write(folder.path().join("documents/00_first.txt"), "earlier\n")
                .expect("seed earlier file");
            open_fixture(&mut host, folder.path());

            // Choosing the already-active folder reveals in place without leaving stale state.
            let root = Location::local(folder.path());
            let same_entry = dummy_archive_at(&same_tar, "same.tar");
            let same_dest = root.clone();
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                extract_to_chosen(app, same_entry, same_dest, tx);
            });
            assert!(
                host.run_until(PATIENCE, |h| is_selected_at(h, &root, "same_dir.txt")),
                "same-folder extract_to_chosen selects same_dir.txt"
            );
            assert!(host.app().archive.pending_destination.is_none());
            assert!(host.app().archive.pending_select.is_empty());

            // Choosing another folder navigates there and preserves pending_select across Reset.
            let documents = Location::local(folder.path().join("documents"));
            let other_entry = dummy_archive_at(&other_tar, "other.tar");
            let docs_dest = documents.clone();
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                extract_to_chosen(app, other_entry, docs_dest, tx);
            });
            assert!(
                host.run_until(PATIENCE, |h| is_selected_at(h, &documents, "into_docs.txt")),
                "cross-folder extract_to_chosen navigates to documents and selects into_docs.txt"
            );
        },
    );
}

#[test]
fn background_completion_from_another_folder_navigates_and_unrelated_reset_clears() {
    hosted(
        MODULE,
        "background_completion_from_another_folder_navigates_and_unrelated_reset_clears",
        |mut host| {
            let folder = fixture();
            std::fs::write(folder.path().join("00_earlier.txt"), "first\n")
                .expect("seed earlier root entry");
            std::fs::write(folder.path().join("zzz_docked.zip"), "docked archive")
                .expect("seed docked archive");
            let documents = folder.path().join("documents");
            open_fixture(&mut host, &documents);

            let root = Location::local(folder.path());
            let dest = root.clone();
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                on_background_completed(app, Some(dest), "zzz_docked.zip".to_owned(), tx);
            });
            assert!(
                host.run_until(PATIENCE, |h| is_selected_at(h, &root, "zzz_docked.zip")),
                "docked completion navigates to root and selects zzz_docked.zip"
            );

            // Navigating to an unrelated folder clears stale pending_destination on Reset.
            let pictures = Location::local(folder.path().join("pictures"));
            let stale = Location::local(documents);
            let target = pictures.clone();
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                app.archive.pending_destination = Some(stale);
                app.archive.pending_select = vec!["ghost.zip".to_owned()];
                app.browser.navigate(target);
                shell::settle(app, tx);
            });
            idle(&mut host);
            assert!(host.app().archive.pending_destination.is_none());
            assert!(host.app().archive.pending_select.is_empty());
        },
    );
}

#[test]
fn several_archives_extract_in_turn_past_a_damaged_one_and_a_mixed_selection_runs_none() {
    hosted(
        MODULE,
        "several_archives_extract_in_turn_past_a_damaged_one_and_a_mixed_selection_runs_none",
        |mut host| {
            let folder = fixture();
            let first = folder.path().join("first.tar");
            write_single_file_tar(&first, "first.txt", b"one\n");
            write_single_file_tar(&folder.path().join("last.tar"), "last.txt", b"two\n");
            std::fs::write(folder.path().join("damaged.zip"), "not a zip").expect("seed damage");
            open_fixture(&mut host, folder.path());

            // An archive selected with a plain file extracts nothing.
            select_names(&mut host, &["first.tar", "readme.md"]);
            run_row_command(&mut host, "extract", &first);
            assert_eq!(host.app().browser().last_started_operation(), None);

            // damaged.zip is first in the listing: its failure does not stop the two after it.
            select_names(&mut host, &["damaged.zip", "first.tar", "last.tar"]);
            run_row_command(&mut host, "extract", &first);
            let revealed = |h: &Offscreen| {
                let selected = selected_names(h);
                h.app().archive.batch.is_none()
                    && ["first.txt", "last.txt"]
                        .iter()
                        .all(|name| selected.iter().any(|selected| selected == name))
            };
            assert!(
                host.run_until(PATIENCE, revealed),
                "both valid archives extract and their results are selected"
            );
            let tree = host.tree();
            assert!(has_a11y_node(&tree, "dialog", "Completed with errors"));
            assert!(mentions(&tree, "damaged.zip"));
        },
    );
}

#[test]
fn a_protected_archive_in_a_batch_prompts_and_closing_the_prompt_starts_no_further_archive() {
    hosted(
        MODULE,
        "a_protected_archive_in_a_batch_prompts_and_closing_the_prompt_starts_no_further_archive",
        |mut host| {
            let folder = fixture();
            let encrypted = folder.path().join("content-encrypted.7z");
            std::fs::copy(
                repo_fixture("tests/qa/fixtures/content-encrypted.7z"),
                &encrypted,
            )
            .expect("seed encrypted 7z");
            let later = folder.path().join("later.txt");
            write_single_file_tar(&folder.path().join("later.tar"), "later.txt", b"after\n");
            open_fixture(&mut host, folder.path());
            let batch = ["content-encrypted.7z", "later.tar"];

            select_names(&mut host, &batch);
            run_row_command(&mut host, "extract", &encrypted);
            assert!(
                host.run_until(PATIENCE, |h| h.app().archive.open),
                "the protected archive asks for its password"
            );
            let asked = host.app().browser().last_started_operation();
            press(&mut host, Key::Escape, Mods::default());
            idle(&mut host);
            assert!(host.app().archive.batch.is_none());
            assert_eq!(host.app().browser().last_started_operation(), asked);
            assert!(!later.exists());

            // With its password the protected archive extracts, and later.tar follows it.
            select_names(&mut host, &batch);
            run_row_command(&mut host, "extract", &encrypted);
            assert!(
                host.run_until(PATIENCE, |h| h.app().archive.open),
                "the protected archive asks again"
            );
            type_text(&mut host, "secret");
            press(&mut host, Key::Enter, Mods::default());
            let protected = folder.path().join("protected.txt");
            assert!(
                host.run_until(PATIENCE, |_| protected.exists() && later.exists()),
                "both archives extract once the password is given"
            );
        },
    );
}

#[test]
fn a_batch_shows_which_archive_it_is_on_and_cancelling_starts_no_further_archive() {
    hosted(
        MODULE,
        "a_batch_shows_which_archive_it_is_on_and_cancelling_starts_no_further_archive",
        |mut host| {
            let folder = fixture();
            let names = ["a.zip", "b.zip", "c.zip"];
            for name in names {
                std::fs::write(folder.path().join(name), "held").expect("seed archive");
            }
            open_fixture(&mut host, folder.path());
            let held = Rc::new(HeldExtractions::default());
            host.app().browser().set_operation_provider(held.clone());
            // The progress bar pulses for as long as it shows, so these wait on what is shown.
            let shows = |host: &mut Offscreen, title: &str| {
                held.report(|request_id| OperationEvent::ArchiveStarted {
                    request_id,
                    total: 0,
                });
                host.run_until(PATIENCE, |h| has_a11y_node(&h.tree(), "dialog", title))
            };

            select_names(&mut host, &names);
            run_row_command(&mut host, "extract", &folder.path().join("a.zip"));
            assert_eq!(held.asked(), 1);
            assert!(shows(&mut host, "Extracting archive 1 of 3"));

            held.report(|request_id| OperationEvent::Extracted {
                request_id,
                first_name: None,
                created: Vec::new(),
            });
            assert!(
                host.run_until(PATIENCE, |_| held.asked() == 2),
                "b.zip starts once a.zip is extracted"
            );
            assert!(shows(&mut host, "Extracting archive 2 of 3"));

            host.app().browser().cancel_file_operation();
            held.report(|request_id| OperationEvent::Cancelled {
                request_id,
                result: CancelledOperation {
                    completed: Vec::new(),
                    failed: Vec::new(),
                    not_attempted: Vec::new(),
                    affected_locations: HashSet::new(),
                },
            });
            let cancelled =
                |h: &Offscreen| has_a11y_node(&h.tree(), "dialog", "Operation cancelled");
            assert!(host.run_until(PATIENCE, cancelled));
            idle(&mut host);
            assert_eq!(held.asked(), 2, "c.zip never starts");
            assert!(host.app().archive.batch.is_none());
        },
    );
}
