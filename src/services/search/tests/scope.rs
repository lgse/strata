// SPDX-License-Identifier: MIT

use super::*;
use crate::services::search::{
    SearchCoverage, SharedIndex, append_index_items, filter_score_normalized, index_filter,
    start_search_session,
};
use std::sync::Arc;

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
fn wildcard_filters_match_basenames_within_the_selected_scope() {
    let fixture = tempfile::tempdir().expect("fixture");
    let root = fixture.path();
    for name in [
        "clip.MOV",
        "IMG_001.MOV",
        "IMG_001.jpg",
        "clip.MOV.bak",
        ".hidden.MOV",
        "album.MOV/nested.txt",
        "album.MOV/deep.MOV",
    ] {
        fixture_file(root, name);
    }
    for recursive in [false, true] {
        let (search, events) = index_filter(root.to_path_buf(), false, recursive);
        for (query, mut expected) in [
            ("*.MOV", vec!["album.MOV", "clip.MOV", "IMG_001.MOV"]),
            ("IMG*", vec!["IMG_001.MOV", "IMG_001.jpg"]),
            ("IMG*.MOV", vec!["IMG_001.MOV"]),
            (
                "*",
                vec![
                    "album.MOV",
                    "clip.MOV",
                    "IMG_001.MOV",
                    "IMG_001.jpg",
                    "clip.MOV.bak",
                ],
            ),
            ("*.MISSING", vec![]),
            (
                "MOV",
                vec!["album.MOV", "clip.MOV", "IMG_001.MOV", "clip.MOV.bak"],
            ),
        ] {
            if recursive {
                match query {
                    "*.MOV" | "MOV" => expected.push("album.MOV/deep.MOV"),
                    "*" => expected.extend(["album.MOV/nested.txt", "album.MOV/deep.MOV"]),
                    _ => {}
                }
            }
            search.query(query);
            let SearchEvent::Results {
                query: returned,
                items,
                indexing,
                coverage,
                ..
            } = wait_for_results(&events).expect("filter results");
            assert_eq!(returned, query);
            assert!(!indexing);
            assert!(!coverage.is_partial());
            let actual: HashSet<_> = items.iter().map(|item| item.path.clone()).collect();
            let expected: HashSet<_> = expected.into_iter().map(|name| root.join(name)).collect();
            assert_eq!(actual, expected, "recursive={recursive}, query={query}");
        }
    }
    let (search, events) = index_filter(root.to_path_buf(), true, false);
    search.query("*.MOV");
    let SearchEvent::Results { items, .. } = wait_for_results(&events).expect("hidden results");
    assert!(items.iter().any(|item| item.name == ".hidden.MOV"));
}

#[test]
fn plain_filters_rank_literal_names_above_typos_and_reject_scattered_or_path_matches() {
    let fixture = tempfile::tempdir().expect("fixture");
    for name in [
        "strata-trash.svg",
        "strata-trahs.svg",
        "strata-sliders-horizontal.svg",
        "strata-refresh.svg",
        "strata-search.svg",
        "strata-list-checks.svg",
        "trash-folder/unrelated.svg",
        "nested/STRATA-TRASH-FULL.svg",
    ] {
        fixture_file(fixture.path(), name);
    }
    for recursive in [false, true] {
        let (search, events) = index_filter(fixture.path().into(), false, recursive);
        for query in ["trash", "trahs"] {
            search.query(query);
            let SearchEvent::Results { items, .. } = wait_for_results(&events).expect("results");
            let actual: HashSet<_> = items.iter().map(|item| item.name.as_str()).collect();
            let mut expected =
                HashSet::from(["strata-trash.svg", "strata-trahs.svg", "trash-folder"]);
            if recursive {
                expected.insert("STRATA-TRASH-FULL.svg");
            }
            assert_eq!(actual, expected, "recursive={recursive}, query={query}");
            let typo_position = items
                .iter()
                .position(|item| item.name == "strata-trahs.svg");
            assert_eq!(
                typo_position,
                Some(if query == "trash" { items.len() - 1 } else { 0 }),
                "literal matches rank first: recursive={recursive}, query={query}",
            );
        }
    }
}

#[test]
fn wildcard_scoring_is_session_local_and_applies_to_new_index_batches() {
    let index = Arc::new(SharedIndex::new());
    let (filter, events) = start_search_session(index.clone(), filter_score_normalized);
    filter.query("*.MOV");
    let SearchEvent::Results { items, .. } = wait_for_results(&events).expect("empty index");
    assert!(items.is_empty());
    let mut batch: Vec<_> = ["clip.MOV", "clip.MOV.bak", "*.MOV", "IMG_001.jpg"]
        .into_iter()
        .map(|name| SearchItem::for_test(PathBuf::from("/fixture").join(name), false))
        .collect();
    append_index_items(&index, &mut batch, false, SearchCoverage::default());
    index.broadcast_change();
    let SearchEvent::Results { items, .. } =
        wait_for_results(&events).expect("incremental results");
    let expected = items;
    assert_eq!(expected.len(), 2);
    filter.query("*.MOV");
    let SearchEvent::Results { items, .. } = wait_for_results(&events).expect("rescored results");
    assert_eq!(items, expected);

    let (global, global_events) = start_search_session(index, fuzzy_score_normalized);
    global.query("*.MOV");
    let SearchEvent::Results { items, .. } =
        wait_for_results(&global_events).expect("global results");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "*.MOV");
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
