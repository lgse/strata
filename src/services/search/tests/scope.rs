// SPDX-License-Identifier: MIT

use super::*;
use crate::services::search::index_filter;

#[test]
fn directory_filter_keeps_immediate_files_and_folders_without_traversing_children() {
    let fixture = tempfile::tempdir().expect("fixture");
    let root = fixture.path();
    for folder in ["needle-folder", "nested/deep", "node_modules"] {
        fs::create_dir_all(root.join(folder)).expect("folder");
    }
    for name in ["needle.txt", ".needle-hidden", "nested/deep/needle.txt"] {
        fs::write(root.join(name), "fixture").expect("file");
    }
    fs::write(root.join(".ignore"), "needle.txt\n").expect("ignore file");
    let (search, events) = index_filter(root.to_path_buf(), false, false);
    search.query("node_modules");
    let SearchEvent::Results { items, .. } =
        wait_for_results(&events).expect("generated folder match");
    assert_eq!(items.len(), 1);
    assert!(items[0].is_directory);

    for show_hidden in [false, true] {
        let (search, events) = index_filter(root.to_path_buf(), show_hidden, false);
        search.query("needle");
        let SearchEvent::Results {
            items, coverage, ..
        } = wait_for_results(&events).expect("results");
        assert!(!coverage.is_partial());
        assert!(items.iter().all(|item| item.path.parent() == Some(root)));
        assert_eq!(items.len(), if show_hidden { 3 } else { 2 });
        assert!(
            items
                .iter()
                .any(|item| item.name == "needle-folder" && item.is_directory)
        );
    }
}

#[test]
fn recursive_and_directory_filters_never_share_the_wrong_scope() {
    let fixture = tempfile::tempdir().expect("fixture");
    fs::create_dir(fixture.path().join("nested")).expect("folder");
    fs::write(fixture.path().join("nested/needle.txt"), "fixture").expect("file");
    let root = fixture.path().to_path_buf();
    let (recursive, recursive_events) = index_filter(root.clone(), false, true);
    let (local, local_events) = index_filter(root.clone(), false, false);
    let (global, _) = index_tree(root, false);
    assert!(!std::sync::Arc::ptr_eq(&recursive.index, &local.index));
    assert!(std::sync::Arc::ptr_eq(&recursive.index, &global.index));
    recursive.query("needle");
    local.query("needle");
    let SearchEvent::Results { items, .. } =
        wait_for_results(&recursive_events).expect("recursive results");
    assert_eq!(items.len(), 1);
    let SearchEvent::Results { items, .. } =
        wait_for_results(&local_events).expect("local results");
    assert!(items.is_empty());
}
