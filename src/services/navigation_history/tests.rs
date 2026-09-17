// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use super::*;

#[test]
fn search_combines_frequency_and_recency() {
    let directory = tempfile::tempdir().expect("history directory");
    let history = NavigationHistory::open(directory.path().join("history.json"));
    let now = 10 * WEEK_SECONDS;
    let frequent = PathBuf::from("/work/frequent");
    let recent = PathBuf::from("/work/recent");

    for _ in 0..20 {
        history.record_at(&frequent, now - WEEK_SECONDS);
    }
    history.record_at(&recent, now - 10);

    let results = history.search_at("", now);
    assert_eq!(results[0].path, frequent);
    assert_eq!(results[1].path, recent);
}

#[test]
fn textual_relevance_remains_stronger_than_frecency() {
    let directory = tempfile::tempdir().expect("history directory");
    let history = NavigationHistory::open(directory.path().join("history.json"));
    let now = 10 * WEEK_SECONDS;
    let exact = PathBuf::from("/work/alpha");
    let frequent_path_match = PathBuf::from("/alpha/archive");
    history.entries.replace(vec![
        HistoryEntry {
            path: exact.clone(),
            rank: 1.0,
            last_accessed: now,
        },
        HistoryEntry {
            path: frequent_path_match,
            rank: 1_000.0,
            last_accessed: now,
        },
    ]);

    let results = history.search_at("alpha", now);
    assert_eq!(results[0].path, exact);
}

#[test]
fn records_round_trip_byte_safe_paths() {
    use std::os::unix::ffi::OsStringExt;

    let directory = tempfile::tempdir().expect("history directory");
    let store = directory.path().join("history.json");
    let path = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/non-utf8-\xff".to_vec()));
    let history = NavigationHistory::open(store.clone());
    history.record_at(&path, WEEK_SECONDS);
    drop(history);

    let loaded = NavigationHistory::open(store);
    let results = loaded.search_at("", WEEK_SECONDS);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].path, path);
}

#[test]
fn invalid_or_unsupported_history_is_ignored() {
    let directory = tempfile::tempdir().expect("history directory");
    let store = directory.path().join("history.json");
    std::fs::write(&store, b"not json").expect("write invalid history");
    assert!(
        NavigationHistory::open(store.clone())
            .entries
            .borrow()
            .is_empty()
    );

    std::fs::write(&store, br#"{"version":99,"entries":[]}"#).expect("write unsupported history");
    assert!(NavigationHistory::open(store).entries.borrow().is_empty());
}

#[test]
fn repeated_visits_update_one_entry() {
    let directory = tempfile::tempdir().expect("history directory");
    let history = NavigationHistory::open(directory.path().join("history.json"));
    let path = PathBuf::from("/work/project");
    history.record_at(&path, 10);
    history.record_at(&path, 20);

    let entries = history.entries.borrow();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].rank, 2.0);
    assert_eq!(entries[0].last_accessed, 20);
}
