// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::services::search::{
    RESULT_LIMIT, SearchCoverage, SharedIndex, append_index_items, insert_match, score_index,
    start_search_session,
};
use std::sync::Arc;

fn ranked_fixture(count: usize) -> Vec<SearchItem> {
    (0..count)
        .map(|position| {
            let name = match position % 5 {
                0 => format!("archive/needle-{position:06}.txt"),
                1 => format!("needle-{position:06}"),
                2 => format!("nested/objects-{position:06}.rs"),
                3 => format!("配置/İstanbul-Σ-{position:06}.txt"),
                _ => format!("ordinary-{position:06}"),
            };
            SearchItem::new(
                Path::new("/fixture").join(name),
                Path::new("/fixture"),
                false,
            )
        })
        .collect()
}

#[test]
fn heap_and_parallel_scoring_match_a_full_stable_sort() {
    let items = ranked_fixture(50_003);
    for query in ["needle", "nested/objects", "ndobj", "配置", "missing", ""] {
        let mut expected: Vec<_> = items
            .iter()
            .filter_map(|item| {
                fuzzy_score_normalized(item, query).map(|score| (score, item.clone()))
            })
            .collect();
        expected.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
        expected.truncate(RESULT_LIMIT);
        assert_eq!(score_index(&items, query), expected, "query: {query}");
    }
}

#[test]
fn incremental_and_completed_queries_keep_the_same_tied_results() {
    let items = ranked_fixture(301);
    let mut incremental = Vec::new();
    for item in &items {
        if let Some(score) = fuzzy_score_normalized(item, "needle") {
            insert_match(&mut incremental, score, item);
        }
    }
    assert_eq!(incremental, score_index(&items, "needle"));
}

#[test]
fn normalized_name_offsets_support_unicode_and_non_utf8_paths() {
    use std::os::unix::ffi::OsStringExt;

    let root = Path::new("/fixture");
    for name in [
        std::ffi::OsString::from("İstanbul-Σ.txt"),
        std::ffi::OsString::from_vec(b"invalid-\xff-name.txt".to_vec()),
    ] {
        let item = SearchItem::new(root.join("配置").join(name), root, false);
        assert_eq!(item.search_name(), item.name.to_lowercase());
        assert!(fuzzy_score_normalized(&item, item.search_name()).is_some());
    }
}

#[test]
fn ascii_fuzzy_scoring_matches_character_scoring_on_utf8_paths() {
    fn reference(haystack: &str, needle: &str) -> Option<i64> {
        let mut chars = haystack.char_indices();
        let mut previous = None;
        let mut score = 1_000;
        for wanted in needle.chars() {
            let (position, _) = chars.find(|(_, candidate)| *candidate == wanted)?;
            score -= position as i64;
            if previous.is_some_and(|previous| previous + 1 == position) {
                score += 80;
            }
            if position == 0
                || haystack[..position]
                    .chars()
                    .next_back()
                    .is_some_and(|character| matches!(character, '/' | '-' | '_' | ' ' | '.'))
            {
                score += 45;
            }
            previous = Some(position);
        }
        Some(score)
    }

    for path in ["配置/needle.txt", "aébc/d_é-e.txt", "nested/object.rs", ""] {
        for query in ["ndlt", "abc", "ae", "obj", "missing", ""] {
            assert_eq!(
                super::super::fuzzy_subsequence_score(path, query),
                reference(path, query),
                "{path}: {query}"
            );
        }
    }
}

#[test]
fn generated_directory_names_do_not_hide_regular_files_or_explicit_roots() {
    let fixture = tempfile::tempdir().expect("fixture");
    for name in ["target", "node_modules", ".cache"] {
        fs::write(fixture.path().join(name), b"user file").expect("fixture file");
    }
    let (search, events) = index_tree(fixture.path().to_path_buf(), true);
    for name in ["target", "node_modules", ".cache"] {
        search.query(name);
        let SearchEvent::Results { items, .. } = wait_for_results(&events).expect("results");
        assert!(items.iter().any(|item| item.name == name));
    }
    drop(search);

    fs::remove_file(fixture.path().join("target")).expect("remove file");
    fs::create_dir(fixture.path().join("target")).expect("explicit root");
    fs::write(fixture.path().join("target/needle.txt"), b"result").expect("fixture file");
    let (search, events) = index_tree(fixture.path().join("target"), true);
    search.query("needle");
    let SearchEvent::Results { items, .. } = wait_for_results(&events).expect("results");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "needle.txt");
}

#[test]
fn priority_pass_preserves_shallow_matches_under_a_shared_budget() {
    let fixture = tempfile::tempdir().expect("fixture");
    fs::create_dir_all(fixture.path().join("archive/deep")).expect("deep directory");
    fs::write(fixture.path().join("needle.txt"), b"result").expect("shallow file");
    for position in 0..20 {
        fs::write(
            fixture
                .path()
                .join(format!("archive/deep/needle-{position}")),
            b"result",
        )
        .expect("deep file");
    }
    let (search, events) = index_tree_with_budget(
        fixture.path().to_path_buf(),
        false,
        3,
        64,
        Duration::from_secs(10),
    );
    search.query("needle");
    let SearchEvent::Results {
        items, coverage, ..
    } = wait_for_results(&events).expect("results");
    assert!(coverage.entry_limit);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "needle.txt");
}

#[test]
fn malformed_ignore_files_report_incomplete_coverage() {
    let fixture = tempfile::tempdir().expect("fixture");
    fs::write(fixture.path().join(".ignore"), "[z-a]\n").expect("malformed ignore file");
    fs::write(fixture.path().join("needle.txt"), b"result").expect("fixture file");
    let (search, events) = index_tree(fixture.path().to_path_buf(), false);
    search.query("needle");
    let SearchEvent::Results {
        items, coverage, ..
    } = wait_for_results(&events).expect("results");
    assert!(coverage.unreadable);
    assert_eq!(items.len(), 1);
}

#[test]
fn releasing_one_session_keeps_the_other_alive_and_last_release_refreshes_the_snapshot() {
    let fixture = tempfile::tempdir().expect("fixture");
    let root = fixture.path().to_path_buf();
    fs::write(root.join("old-needle.txt"), b"old").expect("fixture file");
    let (first, _) = index_tree(root.clone(), false);
    let (second, events) = index_tree(root.clone(), false);
    let retired = first.index.clone();
    assert!(Arc::ptr_eq(&retired, &second.index));
    drop(first);
    assert!(!retired.is_retired());
    second.query("needle");
    let SearchEvent::Results { items, .. } = wait_for_results(&events).expect("results");
    assert_eq!(items.len(), 1);
    drop(second);
    assert!(retired.is_retired());

    fs::remove_file(root.join("old-needle.txt")).expect("remove old file");
    fs::write(root.join("new-needle.txt"), b"new").expect("new file");
    let (fresh, events) = index_tree(root, false);
    assert!(!Arc::ptr_eq(&retired, &fresh.index));
    fresh.query("needle");
    let SearchEvent::Results { items, .. } = wait_for_results(&events).expect("results");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "new-needle.txt");
}

#[test]
fn completed_events_include_the_final_batch_and_coverage() {
    let index = Arc::new(SharedIndex::new());
    let (search, events) = start_search_session(index.clone());
    search.query("needle");
    let SearchEvent::Results { indexing, .. } = events
        .recv_timeout(Duration::from_secs(2))
        .expect("initial query");
    assert!(indexing);
    let coverage = SearchCoverage {
        entry_limit: true,
        ..Default::default()
    };
    let mut batch = ranked_fixture(100);
    let expected = score_index(&batch, "needle");
    append_index_items(&index, &mut batch, false, coverage);
    index.broadcast_change();
    let SearchEvent::Results {
        items,
        indexing,
        coverage: actual_coverage,
        ..
    } = events
        .recv_timeout(Duration::from_secs(2))
        .expect("completion");
    assert!(!indexing);
    assert_eq!(actual_coverage, coverage);
    assert_eq!(
        items,
        expected
            .into_iter()
            .map(|(_, item)| item)
            .collect::<Vec<_>>()
    );

    search.query("   ");
    let SearchEvent::Results {
        query,
        items,
        indexing,
        ..
    } = events
        .recv_timeout(Duration::from_secs(2))
        .expect("cleared query");
    assert!(query.is_empty());
    assert!(items.is_empty());
    assert!(!indexing);
}
