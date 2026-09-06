// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn measurement_counts_nested_entries_without_following_symlinks() {
    let root = tempfile::tempdir().expect("fixture");
    let outside = tempfile::tempdir().expect("outside fixture");
    std::fs::create_dir(root.path().join("nested")).expect("directory");
    std::fs::write(root.path().join("top"), b"abc").expect("top file");
    std::fs::write(root.path().join("nested/child"), b"12345").expect("nested file");
    std::fs::write(outside.path().join("excluded"), b"excluded content").expect("outside file");
    std::os::unix::fs::symlink(outside.path(), root.path().join("link")).expect("symlink");
    let summary = glib::MainContext::new()
        .block_on(summarize_trash(&gio::File::for_path(root.path())))
        .expect("summary");
    assert_eq!(summary.item_count, 4);
    assert_eq!(summary.total_size, 8);
    assert!(!summary.truncated);
}

#[test]
fn missing_root_is_an_error_and_empty_root_is_exact() {
    let root = tempfile::tempdir().expect("fixture");
    let context = glib::MainContext::new();
    assert!(
        context
            .block_on(summarize_trash(&gio::File::for_path(
                root.path().join("missing")
            )))
            .is_err()
    );
    let summary = context
        .block_on(summarize_trash(&gio::File::for_path(root.path())))
        .expect("empty summary");
    assert_eq!(summary.item_count, 0);
    assert_eq!(summary.total_size, 0);
    assert!(!summary.truncated);
}

#[test]
fn measurement_budget_is_shared_by_root_files_and_nested_branches() {
    let root = tempfile::tempdir().expect("fixture");
    std::fs::write(root.path().join("top"), b"top").expect("top file");
    for directory in ["one", "two", "three"] {
        std::fs::create_dir(root.path().join(directory)).expect("directory");
        for index in 0..4 {
            std::fs::write(root.path().join(directory).join(index.to_string()), b"data")
                .expect("file");
        }
    }
    for max_entries in [0, 1, 3, 8] {
        let summary = glib::MainContext::new()
            .block_on(summarize_trash_with_budget(
                &gio::File::for_path(root.path()),
                max_entries,
                MAX_TRASH_DEPTH,
                TRASH_TIME_BUDGET,
            ))
            .expect("bounded summary");
        assert_eq!(summary.item_count, max_entries);
        assert!(summary.truncated);
    }
}

#[test]
fn deletion_caps_errors_and_still_processes_successful_siblings() {
    let root = tempfile::tempdir().expect("fixture");
    for index in 0..12 {
        let directory = root.path().join(format!("directory-{index}"));
        std::fs::create_dir(&directory).expect("directory");
        std::fs::write(directory.join("keep"), b"not empty").expect("child");
    }
    for index in 0..4 {
        std::fs::write(root.path().join(format!("file-{index}")), b"remove").expect("file");
    }
    let mut progress = Vec::new();
    let outcome = glib::MainContext::new()
        .block_on(empty_trash(
            &gio::File::for_path(root.path()),
            |processed| progress.push(processed),
        ))
        .expect("deletion outcome");
    assert_eq!(outcome.deleted, 4);
    assert_eq!(outcome.failed, 12);
    assert_eq!(outcome.errors.len(), 8);
    assert_eq!(progress.last(), Some(&16));
    assert!(progress.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        std::fs::read_dir(root.path())
            .expect("remaining entries")
            .count(),
        12
    );
}
