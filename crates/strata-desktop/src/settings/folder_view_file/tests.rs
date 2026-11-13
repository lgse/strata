// SPDX-License-Identifier: MIT

use std::{fs, io, time::Duration};

use super::*;
use crate::settings::lock_saves;

/// Where a test run in a process of its own says that it ran.
const RAN: &str = "STRATA_TEST_RAN";

/// Runs `run` as the test `name` in a process of its own, with `file` as `folder-views.toml`
/// if there is one, and fails when no test ran there.
///
/// The child is this executable started for the one test whose path is `name` exactly. A
/// path that is no test's starts nothing, and the child still exits well, so without the
/// marker a test moved or renamed passes without running (finding lgse/strata#2193).
pub(in crate::settings) fn isolated(name: &str, file: Option<&str>, run: impl FnOnce()) {
    let own = tempfile::tempdir().expect("a folder for the marker");
    // The parent picks the place, and the child is told it.
    let marker = std::env::var_os(RAN).map_or_else(|| own.path().join("ran"), PathBuf::from);
    strata_test_support::isolated_test(
        name,
        [(RAN, marker.as_os_str())],
        || true,
        || {
            fs::write(&marker, name).expect("the marker");
            if let Some(contents) = file {
                write_file(contents);
            }
            run();
        },
    );
    assert!(
        marker.exists(),
        "`{name}` ran nowhere: no test of this executable has that path, so the child ran none \
         and exited well. Give the test's whole path, with every module it is in."
    );
}

pub(in crate::settings) fn write_file(contents: &str) {
    let path = folder_views_path();
    fs::create_dir_all(path.parent().expect("a state folder")).expect("state folder");
    fs::write(path, contents).expect("folder views fixture");
}

pub(in crate::settings) fn file() -> Option<String> {
    fs::read_to_string(folder_views_path()).ok()
}

/// One folder with an icon size, as the file holds it. A larger `used` is a later use.
pub(in crate::settings) fn folder(path: &str, icons_size: i32, used: u64) -> String {
    format!("[[folder]]\npath = \"{path}\"\nicons_size = {icons_size}\nused = {used}\n\n")
}

pub(in crate::settings) fn views(contents: &str) -> FolderViews {
    let (views, repaired) = FolderViews::parse(contents).expect("a folder views fixture");
    assert!(!repaired, "the fixture needed repair: {contents}");
    views
}

/// What the file holds now, as values.
pub(in crate::settings) fn saved() -> FolderViews {
    views(&file().expect("a saved file"))
}

const MODULE: &str = "settings::folder_view_file::tests";

fn own_process(name: &str, file: Option<&str>, run: impl FnOnce()) {
    isolated(&format!("{MODULE}::{name}"), file, run);
}

#[test]
fn loading_reads_the_file_and_no_file_is_no_values() {
    own_process(
        "loading_reads_the_file_and_no_file_is_no_values",
        None,
        || {
            let (none, failure) = load_folder_views();
            assert!(none.is_empty() && failure.is_none());
            assert_eq!(file(), None, "loading wrote nothing");

            let downloads = folder("/home/me/Downloads", 96, 1);
            write_file(&downloads);
            let (loaded, failure) = load_folder_views();
            assert!(failure.is_none());
            assert!(loaded.same_views(&views(&downloads)));
            assert!(!loaded.has_unsaved(), "what was loaded is saved");
        },
    );
}

#[test]
fn a_file_that_cannot_be_read_loads_as_no_values_with_the_reason() {
    for unreadable in ["folder = [", "version = 99\n"] {
        own_process(
            "a_file_that_cannot_be_read_loads_as_no_values_with_the_reason",
            Some(unreadable),
            || {
                let (views, failure) = load_folder_views();
                assert!(views.is_empty());
                let failure = failure.expect("the reason saving is off");
                assert_eq!(failure.kind(), io::ErrorKind::InvalidData);
            },
        );
    }
}

/// A snapshot with one unsaved change: `/home/me/Pictures` lost its icon size.
fn changed(before: &str) -> FolderViews {
    let mut snapshot = views(before);
    assert!(snapshot.prune_default_icons_size(128));
    assert!(snapshot.has_unsaved());
    snapshot
}

#[test]
fn a_save_writes_the_snapshot_and_keeps_what_another_process_saved() {
    let downloads = folder("/home/me/Downloads", 96, 1);
    let pictures = folder("/home/me/Pictures", 128, 2);
    let ours = format!("{downloads}{pictures}");
    own_process(
        "a_save_writes_the_snapshot_and_keeps_what_another_process_saved",
        None,
        || {
            let path = folder_views_path();
            // No file yet: the snapshot is written as it is.
            let FolderViewsSave::Saved(None) = save_folder_views(&path, views(&ours), false) else {
                panic!("the first save was not written as asked");
            };
            assert!(saved().same_views(&views(&ours)));

            // Another process saved a folder since. This save's own change wins for its
            // folder, the other process's folder is kept, and both come back to adopt.
            let music = folder("/home/me/Music", 64, 3);
            write_file(&format!("{ours}{music}"));
            let FolderViewsSave::Saved(Some(merged)) =
                save_folder_views(&path, changed(&ours), false)
            else {
                panic!("the merged values did not come back");
            };
            let expected = views(&format!("{downloads}{music}"));
            assert!(merged.same_views(&expected));
            assert!(saved().same_views(&expected));
        },
    );
}

#[test]
fn a_save_never_overwrites_a_file_it_cannot_read() {
    let ours = folder("/home/me/Pictures", 128, 1);
    for unreadable in ["folder = [", "version = 99\n"] {
        own_process(
            "a_save_never_overwrites_a_file_it_cannot_read",
            Some(unreadable),
            || {
                let saved = save_folder_views(&folder_views_path(), changed(&ours), false);
                let FolderViewsSave::Failed(error) = saved else {
                    panic!("an unreadable file was saved over");
                };
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                assert_eq!(file().as_deref(), Some(unreadable));
            },
        );
    }
}

#[test]
fn a_save_stands_back_for_another_process_except_at_exit() {
    let ours = folder("/home/me/Pictures", 128, 1);
    own_process(
        "a_save_stands_back_for_another_process_except_at_exit",
        Some(&ours),
        || {
            let path = folder_views_path();
            let other_process = lock_saves(&path, Duration::ZERO).expect("the save lock");
            assert!(matches!(
                save_folder_views(&path, changed(&ours), false),
                FolderViewsSave::Busy
            ));
            assert_eq!(
                file().as_deref(),
                Some(ours.as_str()),
                "nothing was written"
            );

            // At exit the save waits briefly and then goes ahead without the lock.
            assert!(matches!(
                save_folder_views(&path, changed(&ours), true),
                FolderViewsSave::Saved(None)
            ));
            assert!(saved().is_empty());
            drop(other_process);
        },
    );
}
