// SPDX-License-Identifier: MIT

use super::super::*;
use std::{collections::BTreeSet, fs};

fn await_paths(events: &Receiver<SearchEvent>, root: &Path, expected: &[&str]) {
    let expected = expected
        .iter()
        .map(|path| root.join(path))
        .collect::<BTreeSet<_>>();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last = None;
    while Instant::now() < deadline {
        if let Ok(SearchEvent::Results {
            query,
            items,
            indexing,
            ..
        }) = events.recv_timeout(Duration::from_millis(100))
        {
            assert!(!query.is_empty(), "refresh must preserve the active query");
            let paths = items
                .into_iter()
                .map(|item| item.path)
                .collect::<BTreeSet<_>>();
            if !indexing && paths == expected {
                return;
            }
            last = Some(paths);
        }
    }
    panic!("expected {expected:?}, last results {last:?}");
}

#[test]
fn candidate_continuation_returns_lower_ranked_hits_without_changing_default_queries() {
    let fixture = tempfile::tempdir().expect("fixture");
    let root = fixture.path();
    for index in 0..120 {
        fs::write(root.join(format!("needle-{index:03}.txt")), "body").expect("file");
    }
    let last = root.join("needle-picture-with-long-name.png");
    fs::write(&last, "body").expect("file");
    let (handle, events) = index_filter(root.to_path_buf(), false, true);
    handle.query("needle");
    let completed = || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline, "query did not finish");
            if let Ok(SearchEvent::Results {
                items,
                indexing: false,
                has_more,
                ..
            }) = events.recv_timeout(Duration::from_millis(100))
            {
                return (items, has_more);
            }
        }
    };
    let (first, more) = completed();
    assert!(more);
    assert!(!first.iter().any(|item| item.path == last));
    handle.query_candidates("needle", first.len() * 2);
    let (expanded, more) = completed();
    assert!(!more);
    assert!(expanded.iter().any(|item| item.path == last));
    assert!(expanded.starts_with(&first));
    handle.query("needle");
    let (reset, _) = completed();
    assert_eq!(reset, first);
}

#[test]
fn rename_refresh_rescores_every_session_sharing_the_index() {
    let fixture = tempfile::tempdir().expect("fixture");
    let root = fixture.path();
    fs::write(root.join("needle.txt"), "body").expect("file");
    let (first, first_events) = index_filter(root.to_path_buf(), false, true);
    let (second, second_events) = index_filter(root.to_path_buf(), false, true);
    first.query("needle");
    second.query("txt");
    await_paths(&first_events, root, &["needle.txt"]);
    await_paths(&second_events, root, &["needle.txt"]);
    fs::rename(root.join("needle.txt"), root.join("needle-renamed.txt")).expect("rename");
    refresh_search_indexes_for_rename(&root.join("needle.txt"), &root.join("needle-renamed.txt"));
    await_paths(&first_events, root, &["needle-renamed.txt"]);
    await_paths(&second_events, root, &["needle-renamed.txt"]);
    first.query("renamed");
    await_paths(&first_events, root, &["needle-renamed.txt"]);
}

#[test]
fn directory_rename_refresh_preserves_hidden_and_recursive_scope() {
    let fixture = tempfile::tempdir().expect("fixture");
    let root = fixture.path();
    fs::create_dir(root.join(".needle-dir")).expect("directory");
    fs::write(root.join(".needle-dir/needle.txt"), "body").expect("file");
    let sessions = [(false, false), (false, true), (true, true)].map(|(hidden, recursive)| {
        let (handle, events) = index_filter(root.to_path_buf(), hidden, recursive);
        handle.query("needle");
        await_paths(
            &events,
            root,
            if hidden {
                &[".needle-dir", ".needle-dir/needle.txt"]
            } else {
                &[]
            },
        );
        (handle, events, hidden, recursive)
    });
    fs::rename(root.join(".needle-dir"), root.join("needle-dir")).expect("unhide directory");
    refresh_search_indexes_for_rename(&root.join(".needle-dir"), &root.join("needle-dir"));
    for (_, events, _, recursive) in &sessions {
        await_paths(
            events,
            root,
            if *recursive {
                &["needle-dir", "needle-dir/needle.txt"]
            } else {
                &["needle-dir"]
            },
        );
    }
    fs::rename(root.join("needle-dir"), root.join(".needle-dir")).expect("hide directory");
    refresh_search_indexes_for_rename(&root.join("needle-dir"), &root.join(".needle-dir"));
    for (_, events, hidden, _) in &sessions {
        await_paths(
            events,
            root,
            if *hidden {
                &[".needle-dir", ".needle-dir/needle.txt"]
            } else {
                &[]
            },
        );
    }
}

#[test]
fn relocated_multi_root_indexes_follow_later_child_renames_and_new_sessions() {
    for recursive in [false, true] {
        let fixture = tempfile::tempdir().expect("fixture");
        let root = fixture.path();
        fs::create_dir_all(root.join("old/subdir/deep")).expect("directory");
        fs::create_dir(root.join("stable")).expect("directory");
        for name in [
            "old/subdir/needle.txt",
            "old/subdir/deep/needle-child.txt",
            "stable/needle-stable.txt",
        ] {
            fs::write(root.join(name), "body").expect("file");
        }
        let (handle, events) = index_scoped(
            vec![root.join("old/subdir"), root.join("stable")],
            false,
            recursive,
            fuzzy_score_normalized,
        );
        handle.query("needle");
        let expected = if recursive {
            vec![
                "old/subdir/needle.txt",
                "old/subdir/deep/needle-child.txt",
                "stable/needle-stable.txt",
            ]
        } else {
            vec!["old/subdir/needle.txt", "stable/needle-stable.txt"]
        };
        await_paths(&events, root, &expected);
        fs::rename(root.join("old"), root.join("new")).expect("rename ancestor");
        refresh_search_indexes_for_rename(&root.join("old"), &root.join("new"));
        let moved = expected
            .iter()
            .map(|path| path.replace("old/", "new/"))
            .collect::<Vec<_>>();
        await_paths(
            &events,
            root,
            &moved.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let (second, second_events) = index_scoped(
            vec![root.join("new/subdir"), root.join("stable")],
            false,
            recursive,
            fuzzy_score_normalized,
        );
        assert!(
            Arc::ptr_eq(&handle.index, &second.index),
            "relocated scope should remain shareable"
        );
        second.query("needle");
        await_paths(
            &second_events,
            root,
            &moved.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        fs::rename(
            root.join("new/subdir/needle.txt"),
            root.join("new/subdir/needle-renamed.txt"),
        )
        .expect("rename child");
        refresh_search_indexes_for_rename(
            &root.join("new/subdir/needle.txt"),
            &root.join("new/subdir/needle-renamed.txt"),
        );
        let renamed = moved
            .iter()
            .map(|path| path.replace("subdir/needle.txt", "subdir/needle-renamed.txt"))
            .collect::<Vec<_>>();
        let expected = renamed.iter().map(String::as_str).collect::<Vec<_>>();
        await_paths(&events, root, &expected);
        await_paths(&second_events, root, &expected);
    }
}

#[test]
fn queued_refreshes_coalesce_to_the_latest_filesystem_state() {
    let fixture = tempfile::tempdir().expect("fixture");
    let root = fixture.path();
    fs::write(root.join("needle.txt"), "body").expect("file");
    let (handle, events) = index_filter(root.to_path_buf(), false, true);
    handle.query("needle");
    await_paths(&events, root, &["needle.txt"]);
    let traversal = REFRESH_TRAVERSAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for (from, to) in [
        ("needle.txt", "needle-next.txt"),
        ("needle-next.txt", "needle-final.txt"),
    ] {
        fs::rename(root.join(from), root.join(to)).expect("rename");
        refresh_search_indexes_for_rename(&root.join(from), &root.join(to));
    }
    drop(traversal);
    await_paths(&events, root, &["needle-final.txt"]);
    handle.query("final");
    await_paths(&events, root, &["needle-final.txt"]);
}

#[test]
fn refreshed_index_rejects_late_original_batches_and_rescores_smaller_snapshots() {
    let fixture = tempfile::tempdir().expect("fixture");
    let root = fixture.path();
    fs::write(root.join("needle-renamed.txt"), "body").expect("file");
    let index = Arc::new(SharedIndex::new());
    let mut old_batch = vec![
        SearchItem::new(root.join("needle.txt"), root, false),
        SearchItem::new(root.join("needle-stale.txt"), root, false),
    ];
    append_index_items(&index, &mut old_batch, false, SearchCoverage::default());
    let (handle, events) = start_search_session(index.clone(), fuzzy_score_normalized);
    handle.query("needle");
    await_paths(&events, root, &["needle.txt", "needle-stale.txt"]);
    request_index_refresh(index.clone(), vec![root.to_path_buf()], false, true);
    await_paths(&events, root, &["needle-renamed.txt"]);
    let mut late_batch = vec![SearchItem::new(root.join("needle.txt"), root, false)];
    append_index_items(&index, &mut late_batch, false, SearchCoverage::default());
    index.broadcast_change();
    await_paths(&events, root, &["needle-renamed.txt"]);
    handle.query("renamed");
    await_paths(&events, root, &["needle-renamed.txt"]);
}
