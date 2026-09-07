// SPDX-License-Identifier: GPL-3.0-or-later

mod multi_root;
mod performance;

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use super::{
    SearchEvent, SearchItem, fuzzy_score_normalized, index_tree, index_trees,
    index_trees_with_budget,
};

fn score_path(path: &str, query: &str, root: &Path) -> Option<i64> {
    fuzzy_score_normalized(&SearchItem::new(PathBuf::from(path), root, false), query)
}

fn index_tree_with_budget(
    root: PathBuf,
    show_hidden: bool,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
) -> (super::SearchHandle, std::sync::mpsc::Receiver<SearchEvent>) {
    index_trees_with_budget(vec![root], show_hidden, max_entries, max_depth, time_budget)
}

#[test]
fn exact_names_rank_above_substrings_and_fuzzy_matches() {
    let root = Path::new("/home/me");
    let exact = score_path("/home/me/notes", "notes", root).expect("an exact name should match");
    let substring =
        score_path("/home/me/my-notes.txt", "notes", root).expect("a name substring should match");
    let fuzzy = score_path("/home/me/nested-object-types.rs", "notes", root)
        .expect("an ordered fuzzy subsequence should match");
    assert!(exact > substring);
    assert!(substring > fuzzy);
}

#[test]
fn nearby_duplicate_names_rank_first_without_overriding_match_quality() {
    let root = Path::new("/fixture/Videos");
    let score = |path, query| score_path(path, query, root).expect("fixture should match");
    assert!(
        score("/fixture/Videos/recording.mp4", "recording")
            > score("/fixture/Videos/archive/recording.mp4", "recording")
    );
    assert!(
        score("/fixture/Videos/archive/recording.mp4", "recording.mp4")
            > score("/fixture/Videos/old-recording.mp4", "recording.mp4")
    );
}

#[test]
fn recursive_results_stay_in_the_root_and_rank_nearby_duplicates_first() {
    let fixture = unique_fixture_root("nearby-results");
    let root = fixture.join("Videos");
    fs::create_dir_all(root.join("archive/deep")).expect("create nested fixture");
    for path in [
        fixture.join("recording.mp4"),
        root.join("recording.mp4"),
        root.join("archive/recording.mp4"),
        root.join("archive/deep/recording.mp4"),
    ] {
        fs::write(path, b"fixture").expect("create matching file");
    }
    let (search, events) = index_tree(root.clone(), false);
    search.query("recording.mp4");
    let SearchEvent::Results { items, .. } =
        wait_for_results(&events).expect("search should return results");
    assert_eq!(
        items
            .iter()
            .map(|item| item.path.clone())
            .collect::<Vec<_>>(),
        vec![
            root.join("recording.mp4"),
            root.join("archive/recording.mp4"),
            root.join("archive/deep/recording.mp4"),
        ]
    );
    drop(search);
    fs::remove_dir_all(fixture).expect("remove fixture");
}

#[test]
fn completed_index_returns_only_the_best_bounded_matches() {
    let root = unique_fixture_root("bounded-best-matches");
    fs::create_dir_all(&root).expect("create fixture");
    fs::write(root.join("needle"), b"best match").expect("write exact match");
    for position in 0..120 {
        fs::write(root.join(format!("needle-{position:03}")), b"candidate")
            .expect("write candidate");
    }

    let (search, events) = index_tree(root.clone(), false);
    let SearchEvent::Results { indexing, .. } = events
        .recv_timeout(Duration::from_secs(2))
        .expect("index completion");
    assert!(!indexing);
    search.query("needle");
    let event = wait_for_results(&events);

    drop(search);
    fs::remove_dir_all(&root).expect("remove fixture");

    let Some(SearchEvent::Results { items, .. }) = event else {
        panic!("the worker should publish bounded results");
    };
    assert_eq!(items.len(), 100);
    assert_eq!(items.first().map(|item| item.name.as_str()), Some("needle"));
}

#[test]
fn searches_of_the_same_tree_share_an_index_but_keep_independent_queries() {
    let root = unique_fixture_root("shared-index");
    fs::create_dir_all(&root).expect("create fixture");
    fs::write(root.join("alpha-only"), b"alpha").expect("write alpha fixture");
    fs::write(root.join("beta-only"), b"beta").expect("write beta fixture");

    let (alpha_search, alpha_events) = index_tree(root.clone(), false);
    let (beta_search, beta_events) = index_tree(root.clone(), false);
    assert!(std::sync::Arc::ptr_eq(
        &alpha_search.index,
        &beta_search.index
    ));
    alpha_search.query("alpha-only");
    beta_search.query("beta-only");
    let alpha_event = wait_for_results(&alpha_events);
    let beta_event = wait_for_results(&beta_events);

    drop((alpha_search, beta_search));
    fs::remove_dir_all(&root).expect("remove fixture");

    let Some(SearchEvent::Results {
        items: alpha_items, ..
    }) = alpha_event
    else {
        panic!("the alpha session should publish results");
    };
    let Some(SearchEvent::Results {
        items: beta_items, ..
    }) = beta_event
    else {
        panic!("the beta session should publish results");
    };
    assert_eq!(
        alpha_items.first().map(|item| item.name.as_str()),
        Some("alpha-only")
    );
    assert_eq!(
        beta_items.first().map(|item| item.name.as_str()),
        Some("beta-only")
    );
}

#[test]
fn searches_relative_path_fragments_and_rejects_non_matches() {
    let candidate = "/home/me/themes/azure/colors.toml";
    assert!(score_path(candidate, "themes/azure", Path::new("/home/me")).is_some());
    assert!(score_path(candidate, "definitely-missing", Path::new("/home/me")).is_none());
}

#[test]
fn background_index_returns_results_for_queries_received_while_walking() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-search-{unique}"));
    fs::create_dir_all(root.join("nested")).expect("the search fixture should be created");
    fs::write(root.join("nested/needle.txt"), b"result")
        .expect("the search fixture file should be written");

    let (search, events) = index_tree(root.clone(), false);
    search.query("needle");
    let found = (0..20).any(|_| {
        events.recv_timeout(Duration::from_millis(100)).is_ok_and(
            |SearchEvent::Results { query, items, .. }| {
                query == "needle" && items.iter().any(|item| item.name == "needle.txt")
            },
        )
    });

    drop(search);
    fs::remove_dir_all(root).expect("the search fixture should be removed");
    assert!(found, "the worker should publish the matching indexed file");
}

#[test]
fn hidden_files_are_indexed_only_when_show_hidden_is_enabled() {
    let root = unique_fixture_root("hidden-files");
    fs::create_dir_all(&root).expect("the search fixture should be created");
    fs::write(root.join(".dotfile-needle"), b"content")
        .expect("the hidden fixture file should be written");

    let (search, events) = index_tree(root.clone(), false);
    search.query("needle");
    let event = wait_for_results(&events);
    drop(search);
    let Some(SearchEvent::Results { items, .. }) = event else {
        panic!("the worker should publish a result for a non-empty query");
    };
    assert!(
        items.is_empty(),
        "a hidden file should not match while hidden files are not shown"
    );

    let (search, events) = index_tree(root.clone(), true);
    search.query("needle");
    let event = wait_for_results(&events);
    drop(search);
    fs::remove_dir_all(&root).expect("the search fixture should be removed");
    let Some(SearchEvent::Results { items, .. }) = event else {
        panic!("the worker should publish a result for a non-empty query");
    };
    assert!(
        items.iter().any(|item| item.name == ".dotfile-needle"),
        "a visible hidden file should match once hidden files are shown"
    );
}

#[test]
fn generated_tool_content_is_pruned_without_hiding_tool_configuration() {
    let root = unique_fixture_root("tool-content");
    fs::create_dir_all(root.join(".cargo/registry")).expect("create Cargo registry fixture");
    fs::create_dir_all(root.join(".m2/repository")).expect("create Maven repository fixture");
    fs::write(root.join(".cargo/config-needle.toml"), b"[build]")
        .expect("write Cargo configuration fixture");
    fs::write(root.join(".m2/settings-needle.xml"), b"<settings />")
        .expect("write Maven configuration fixture");
    fs::write(root.join(".cargo/registry/registry-needle"), b"generated")
        .expect("write generated Cargo fixture");
    fs::write(root.join(".m2/repository/artifact-needle"), b"generated")
        .expect("write generated Maven fixture");

    let (search, events) = index_tree(root.clone(), true);
    search.query("needle");
    let event = wait_for_results(&events);

    drop(search);
    fs::remove_dir_all(&root).expect("remove fixture");

    let Some(SearchEvent::Results { items, .. }) = event else {
        panic!("the worker should publish tool configuration results");
    };
    let names = items
        .iter()
        .map(|item| item.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"config-needle.toml"));
    assert!(names.contains(&"settings-needle.xml"));
    assert!(!names.contains(&"registry-needle"));
    assert!(!names.contains(&"artifact-needle"));
}

#[test]
fn index_reports_completion_before_a_query_is_entered() {
    let root = unique_fixture_root("empty-query-completion");
    fs::create_dir_all(&root).expect("the search fixture should be created");

    let (search, events) = index_tree(root.clone(), false);
    let event = events
        .recv_timeout(Duration::from_secs(2))
        .expect("index completion should be published without a query");

    drop(search);
    fs::remove_dir_all(&root).expect("the search fixture should be removed");

    assert!(matches!(
        event,
        SearchEvent::Results {
            query,
            indexing: false,
            ..
        } if query.is_empty()
    ));
}

fn unique_fixture_root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("strata-search-{label}-{unique}"))
}

fn wait_for_results(receiver: &std::sync::mpsc::Receiver<SearchEvent>) -> Option<SearchEvent> {
    let mut latest = None;
    for _ in 0..40 {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => latest = Some(event),
            Err(_) if latest.is_some() => break,
            Err(_) => continue,
        }
    }
    latest
}

#[test]
fn index_reports_truncated_once_the_entry_budget_is_exceeded() {
    let root = unique_fixture_root("entry-budget");
    fs::create_dir_all(&root).expect("the search fixture should be created");
    for index in 0..5 {
        fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the search fixture file should be written");
    }

    let (search, events) =
        index_tree_with_budget(root.clone(), false, 2, 64, Duration::from_secs(10));
    search.query("file");
    let event = wait_for_results(&events);

    drop(search);
    fs::remove_dir_all(&root).expect("the search fixture should be removed");

    let Some(SearchEvent::Results {
        coverage, items, ..
    }) = event
    else {
        panic!("the worker should publish a result for a non-empty query");
    };
    assert_eq!(
        coverage,
        super::SearchCoverage {
            entry_limit: true,
            ..Default::default()
        }
    );
    assert!(
        items.len() <= 2,
        "the index should stop growing once the entry budget is reached"
    );
}

#[test]
fn index_reports_truncated_once_the_time_budget_is_exceeded() {
    let root = unique_fixture_root("time-budget");
    fs::create_dir_all(&root).expect("the search fixture should be created");
    fs::write(root.join("needle.txt"), b"content")
        .expect("the search fixture file should be written");
    fs::write(root.join("second.txt"), b"content")
        .expect("the search fixture file should be written");

    let (search, events) =
        index_tree_with_budget(root.clone(), false, usize::MAX, 64, Duration::from_nanos(1));
    search.query("needle");
    let event = wait_for_results(&events);

    drop(search);
    fs::remove_dir_all(&root).expect("the search fixture should be removed");

    let Some(SearchEvent::Results { coverage, .. }) = event else {
        panic!("the worker should publish a result for a non-empty query");
    };
    assert_eq!(
        coverage,
        super::SearchCoverage {
            time_limit: true,
            ..Default::default()
        }
    );
}

#[test]
fn index_does_not_descend_past_the_depth_budget() {
    let root = unique_fixture_root("depth-budget");
    fs::create_dir_all(root.join("nested")).expect("the search fixture should be created");
    fs::write(root.join("shallow-needle.txt"), b"content")
        .expect("the shallow fixture file should be written");
    fs::write(root.join("nested/deep-needle.txt"), b"content")
        .expect("the deep fixture file should be written");

    let (search, events) =
        index_tree_with_budget(root.clone(), false, usize::MAX, 1, Duration::from_secs(10));
    search.query("needle");
    let event = wait_for_results(&events);

    drop(search);
    fs::remove_dir_all(&root).expect("the search fixture should be removed");

    let Some(SearchEvent::Results {
        items, coverage, ..
    }) = event
    else {
        panic!("the worker should publish a result for a non-empty query");
    };
    assert!(
        items.iter().any(|item| item.name == "shallow-needle.txt"),
        "entries within the depth budget should still be indexed"
    );
    assert!(
        items.iter().all(|item| item.name != "deep-needle.txt"),
        "entries past the depth budget should not be indexed"
    );
    assert_eq!(
        coverage,
        super::SearchCoverage {
            depth_limit: true,
            ..Default::default()
        }
    );
}

#[test]
fn index_reports_truncated_when_the_walker_discards_an_inaccessible_directory() {
    use std::os::unix::fs::PermissionsExt;

    let root = unique_fixture_root("inaccessible");
    fs::create_dir_all(root.join("blocked")).expect("the search fixture should be created");
    fs::create_dir_all(root.join("visible")).expect("the search fixture should be created");
    fs::write(root.join("visible/needle.txt"), b"content")
        .expect("the search fixture file should be written");
    fs::set_permissions(root.join("blocked"), fs::Permissions::from_mode(0o000))
        .expect("the fixture directory's permissions should be restrictable");
    let running_as_root = fs::read_dir(root.join("blocked")).is_ok();

    let (search, events) =
        index_tree_with_budget(root.clone(), false, usize::MAX, 64, Duration::from_secs(10));
    search.query("needle");
    let event = wait_for_results(&events);

    drop(search);
    let _ = fs::set_permissions(root.join("blocked"), fs::Permissions::from_mode(0o755));
    fs::remove_dir_all(&root).expect("the search fixture should be removed");

    let Some(SearchEvent::Results {
        items, coverage, ..
    }) = event
    else {
        panic!("the worker should publish a result for a non-empty query");
    };
    assert!(
        items.iter().any(|item| item.name == "needle.txt"),
        "entries outside the inaccessible directory should still be indexed"
    );
    if !running_as_root {
        assert_eq!(
            coverage,
            super::SearchCoverage {
                unreadable: true,
                ..Default::default()
            }
        );
        assert_eq!(
            coverage.message(),
            "Partial search — some folders could not be read"
        );
    }
}
