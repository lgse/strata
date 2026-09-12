// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn measurement_counts_nested_entries_without_following_symlinks() {
    let root = tempfile::tempdir().expect("fixture");
    let outside = tempfile::tempdir().expect("outside fixture");
    std::fs::create_dir(root.path().join("nested")).expect("directory");
    std::fs::write(root.path().join("top"), b"abc").expect("top file");
    std::fs::write(root.path().join("nested/child"), b"12345").expect("nested file");
    std::fs::write(root.path().join("nested/.hidden"), b"hidden").expect("hidden file");
    std::os::unix::fs::symlink(root.path(), root.path().join("nested/loop")).expect("symlink loop");
    std::os::unix::fs::symlink(root.path().join("missing"), root.path().join("broken"))
        .expect("broken symlink");
    std::fs::write(outside.path().join("excluded"), b"excluded content").expect("outside file");
    std::os::unix::fs::symlink(outside.path(), root.path().join("link")).expect("symlink");
    std::fs::create_dir_all(root.path().join(".hidden/visible")).expect("hidden subtree");
    std::fs::write(root.path().join(".hidden/visible/file"), b"1234").expect("hidden subtree file");
    let summary = glib::MainContext::new()
        .block_on(summarize_directory(&gio::File::for_path(root.path())))
        .expect("summary");
    assert_eq!(summary.item_count, 10);
    assert_eq!(summary.total_size, 18);
    assert_eq!(summary.visible_file_count, 5);
    assert_eq!(summary.visible_folder_count, 1);
    assert!(!summary.truncated());
}

#[test]
fn missing_root_is_an_error_and_empty_root_is_exact() {
    let root = tempfile::tempdir().expect("fixture");
    let context = glib::MainContext::new();
    assert!(
        context
            .block_on(summarize_directory(&gio::File::for_path(
                root.path().join("missing")
            )))
            .is_err()
    );
    let summary = context
        .block_on(summarize_directory(&gio::File::for_path(root.path())))
        .expect("empty summary");
    assert_eq!(summary.item_count, 0);
    assert_eq!(summary.total_size, 0);
    assert!(!summary.truncated());
}
