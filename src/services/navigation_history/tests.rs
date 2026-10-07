// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

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

#[test]
fn recent_orders_matches_by_last_visit_not_frequency() {
    let directory = tempfile::tempdir().expect("history directory");
    let history = NavigationHistory::open(directory.path().join("history.json"));
    let frequent = PathBuf::from("/work/alpha-frequent");
    let latest = PathBuf::from("/work/alpha-latest");
    let unrelated = PathBuf::from("/work/beta");
    for visit in 0..20 {
        history.record_at(&frequent, 100 + visit);
    }
    history.record_at(&unrelated, 150);
    history.record_at(&latest, 200);

    let paths = |query| {
        history
            .recent_excluding(query, None)
            .into_iter()
            .map(|item| item.path)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        paths(""),
        vec![latest.clone(), unrelated.clone(), frequent.clone()]
    );
    assert_eq!(paths("alpha"), vec![latest, frequent]);
    assert!(paths("zzz").is_empty());
}

#[test]
fn excluded_folder_does_not_consume_a_result_slot() {
    let directory = tempfile::tempdir().expect("history directory");
    let history = NavigationHistory::open(directory.path().join("history.json"));
    let excluded = PathBuf::from("/work/current");
    for index in 0..MAX_RESULTS {
        history.record_at(
            &PathBuf::from(format!("/work/other-{index:03}")),
            index as u64,
        );
    }
    history.record_at(&excluded, MAX_RESULTS as u64);

    for paths in [
        history.search_excluding("", Some(&excluded)),
        history.recent_excluding("", Some(&excluded)),
    ] {
        assert_eq!(paths.len(), MAX_RESULTS);
        assert!(paths.iter().all(|item| item.path != excluded));
        assert!(
            paths
                .iter()
                .any(|item| item.path == Path::new("/work/other-000"))
        );
    }
}

#[test]
fn frecency_snapshot_weighs_visits_by_recency_below_the_root() {
    let directory = tempfile::tempdir().expect("history directory");
    let history = NavigationHistory::open(directory.path().join("history.json"));
    let now = 10 * WEEK_SECONDS;
    history.record_at(Path::new("/work/stale"), now - 2 * WEEK_SECONDS);
    history.record_at(Path::new("/work/fresh"), now - 10);
    history.record_at(Path::new("/elsewhere/fresh"), now - 10);

    let frecency = history.frecency_within_at(Path::new("/work"), now);
    let stale = frecency.bias(Path::new("/work/stale"), true);
    let fresh = frecency.bias(Path::new("/work/fresh"), true);
    assert!(fresh > stale && stale > 0);
    assert_eq!(frecency.bias(Path::new("/elsewhere/fresh"), true), 0);
}

#[test]
fn jump_queries_take_fzf_terms_across_the_whole_path() {
    let directory = tempfile::tempdir().expect("history directory");
    let history = NavigationHistory::open(directory.path().join("history.json"));
    let now = 10 * WEEK_SECONDS;
    let strata = PathBuf::from("/home/me/dev/strata");
    let source = PathBuf::from("/home/me/dev/strata/src");
    let stratagem = PathBuf::from("/home/me/stratagem");
    let notes = PathBuf::from("/home/me/notes/strata-notes");
    for (path, visits) in [(&strata, 1), (&source, 30), (&stratagem, 30), (&notes, 30)] {
        for _ in 0..visits {
            history.record_at(path, now - 10);
        }
    }

    let jump = |query| {
        history
            .search_at(query, now)
            .into_iter()
            .map(|item| item.path)
            .collect::<Vec<_>>()
    };
    assert_eq!(jump("strata")[0], strata, "an exact name beats frecency");
    assert_eq!(jump("strata dev"), [strata.clone(), source.clone()]);
    assert_eq!(jump("dev src"), [source]);
    let excluded = jump("strata !notes");
    assert_eq!(excluded.len(), 3);
    assert!(!excluded.contains(&notes));
    assert_eq!(jump("^/home/me/notes"), std::slice::from_ref(&notes));
    assert!(!jump("sta").is_empty());
    assert!(jump("'sta").is_empty());
    let recent = history
        .recent_excluding("notes strata", None)
        .into_iter()
        .map(|item| item.path)
        .collect::<Vec<_>>();
    assert_eq!(recent, [notes]);
}
