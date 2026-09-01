// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use super::{SearchEvent, SearchItem, fuzzy_score, index_tree, index_tree_with_budget};

fn item(path: &str) -> SearchItem {
    let name = Path::new(path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    SearchItem {
        path: PathBuf::from(path),
        search_name: name.to_lowercase(),
        search_path: path.to_lowercase(),
        name,
        is_directory: false,
    }
}

#[test]
fn exact_names_rank_above_substrings_and_fuzzy_matches() {
    let root = Path::new("/home/me");
    let exact =
        fuzzy_score(&item("/home/me/notes"), "notes", root).expect("an exact name should match");
    let substring = fuzzy_score(&item("/home/me/my-notes.txt"), "notes", root)
        .expect("a name substring should match");
    let fuzzy = fuzzy_score(&item("/home/me/nested-object-types.rs"), "notes", root)
        .expect("an ordered fuzzy subsequence should match");
    assert!(exact > substring);
    assert!(substring > fuzzy);
}

#[test]
fn searches_relative_path_fragments_and_rejects_non_matches() {
    let candidate = item("/home/me/themes/azure/colors.toml");
    assert!(fuzzy_score(&candidate, "themes/azure", Path::new("/home/me")).is_some());
    assert!(fuzzy_score(&candidate, "definitely-missing", Path::new("/home/me")).is_none());
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

    let (search, events) = index_tree(root.clone());
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
fn index_reports_completion_before_a_query_is_entered() {
    let root = unique_fixture_root("empty-query-completion");
    fs::create_dir_all(&root).expect("the search fixture should be created");

    let (search, events) = index_tree(root.clone());
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

    let (search, events) = index_tree_with_budget(root.clone(), 2, 64, Duration::from_secs(10));
    search.query("file");
    let event = wait_for_results(&events);

    drop(search);
    fs::remove_dir_all(&root).expect("the search fixture should be removed");

    let Some(SearchEvent::Results {
        truncated, items, ..
    }) = event
    else {
        panic!("the worker should publish a result for a non-empty query");
    };
    assert!(truncated, "exceeding the entry budget should be reported");
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
        index_tree_with_budget(root.clone(), usize::MAX, 64, Duration::from_nanos(1));
    search.query("needle");
    let event = wait_for_results(&events);

    drop(search);
    fs::remove_dir_all(&root).expect("the search fixture should be removed");

    let Some(SearchEvent::Results { truncated, .. }) = event else {
        panic!("the worker should publish a result for a non-empty query");
    };
    assert!(
        truncated,
        "an exhausted time budget should stop the walk and report truncation"
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
        index_tree_with_budget(root.clone(), usize::MAX, 1, Duration::from_secs(10));
    search.query("needle");
    let event = wait_for_results(&events);

    drop(search);
    fs::remove_dir_all(&root).expect("the search fixture should be removed");

    let Some(SearchEvent::Results {
        items, truncated, ..
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
    assert!(
        truncated,
        "omitting entries past the depth budget should be reported, not silently look complete"
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
        index_tree_with_budget(root.clone(), usize::MAX, 64, Duration::from_secs(10));
    search.query("needle");
    let event = wait_for_results(&events);

    drop(search);
    let _ = fs::set_permissions(root.join("blocked"), fs::Permissions::from_mode(0o755));
    fs::remove_dir_all(&root).expect("the search fixture should be removed");

    let Some(SearchEvent::Results {
        items, truncated, ..
    }) = event
    else {
        panic!("the worker should publish a result for a non-empty query");
    };
    assert!(
        items.iter().any(|item| item.name == "needle.txt"),
        "entries outside the inaccessible directory should still be indexed"
    );
    if !running_as_root {
        assert!(
            truncated,
            "an inaccessible directory discarded by the walker should be reported as truncated"
        );
    }
}
