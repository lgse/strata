// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn fixture_file(root: &Path, relative: &str) -> PathBuf {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture directory");
    fs::write(&path, b"fixture").expect("fixture file");
    path
}

#[test]
fn global_results_combine_roots_and_rank_depth_relative_to_each_root() {
    let fixture = tempfile::tempdir().expect("fixture");
    let home = fixture.path().join("home");
    let usb = fixture.path().join("run/media/user/USB");
    let home_match = fixture_file(&home, "needle.txt");
    let usb_match = fixture_file(&usb, "needle.txt");
    let deep_match = fixture_file(&usb, "archive/needle.txt");
    fixture_file(fixture.path(), "outside-needle.txt");
    let (search, events) = index_trees(vec![home, usb], false);
    search.query("NEEDLE");
    let SearchEvent::Results {
        query,
        items,
        indexing,
        coverage,
    } = wait_for_results(&events).expect("results");
    assert_eq!(query, "NEEDLE");
    assert!(!indexing);
    assert!(!coverage.is_partial());
    assert_eq!(items.len(), 3);
    assert!(items[..2].iter().any(|item| item.path == home_match));
    assert!(items[..2].iter().any(|item| item.path == usb_match));
    assert_eq!(items[2].path, deep_match);
}

#[test]
fn duplicate_and_nested_roots_produce_each_path_once() {
    let fixture = tempfile::tempdir().expect("fixture");
    let home = fixture.path().join("home");
    let usb = home.join("USB");
    let nested = usb.join("nested");
    let expected = [
        fixture_file(&home, "needle.txt"),
        fixture_file(&usb, "needle.txt"),
        fixture_file(&nested, "needle.txt"),
    ];
    let (search, events) = index_trees(vec![home.clone(), usb, home, nested], false);
    search.query("needle");
    let SearchEvent::Results {
        items, coverage, ..
    } = wait_for_results(&events).expect("results");
    assert!(!coverage.is_partial());
    assert_eq!(items.len(), expected.len());
    for path in expected {
        assert_eq!(items.iter().filter(|item| item.path == path).count(), 1);
    }
}

#[test]
fn all_drives_get_a_turn_before_a_large_home_consumes_the_shared_entry_budget() {
    let fixture = tempfile::tempdir().expect("fixture");
    let home = fixture.path().join("home");
    let usb = fixture.path().join("USB");
    let backup = fixture.path().join("Backup");
    for index in 0..50 {
        fixture_file(&home, &format!("needle-{index}.txt"));
    }
    let usb_match = fixture_file(&usb, "needle.txt");
    let backup_match = fixture_file(&backup, "needle.txt");
    let (search, events) = index_trees_with_budget(
        vec![home, usb, backup],
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
    assert_eq!(items.len(), 3);
    assert!(items.iter().any(|item| item.path == usb_match));
    assert!(items.iter().any(|item| item.path == backup_match));
}

#[test]
fn missing_drive_does_not_discard_home_results_or_claim_full_coverage() {
    let fixture = tempfile::tempdir().expect("fixture");
    let home_match = fixture_file(fixture.path(), "home/needle.txt");
    let (search, events) = index_trees(
        vec![
            fixture.path().join("removed-drive"),
            fixture.path().join("home"),
        ],
        false,
    );
    search.query("needle");
    let SearchEvent::Results {
        items, coverage, ..
    } = wait_for_results(&events).expect("results");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].path, home_match);
    assert!(coverage.unreadable);
    assert!(!coverage.entry_limit);
}

#[test]
fn hidden_file_visibility_applies_to_every_root() {
    let fixture = tempfile::tempdir().expect("fixture");
    let roots = [fixture.path().join("home"), fixture.path().join("USB")];
    for root in &roots {
        fixture_file(root, "visible-needle.txt");
        fixture_file(root, ".hidden-needle.txt");
    }
    for (show_hidden, expected) in [(false, 2), (true, 4)] {
        let (search, events) = index_trees(roots.to_vec(), show_hidden);
        search.query("needle");
        let SearchEvent::Results { items, .. } = wait_for_results(&events).expect("results");
        assert_eq!(items.len(), expected);
    }
}

#[test]
fn result_limit_is_shared_across_drives() {
    let fixture = tempfile::tempdir().expect("fixture");
    let roots = [fixture.path().join("home"), fixture.path().join("USB")];
    for root in &roots {
        for index in 0..80 {
            fixture_file(root, &format!("needle-{index}.txt"));
        }
    }
    let (search, events) = index_trees(roots.to_vec(), false);
    search.query("needle");
    let SearchEvent::Results {
        items, coverage, ..
    } = wait_for_results(&events).expect("results");
    assert_eq!(items.len(), super::super::RESULT_LIMIT);
    assert!(!coverage.is_partial());
    assert!(
        roots
            .iter()
            .all(|root| items.iter().any(|item| item.path.starts_with(root)))
    );
}

#[test]
fn dropping_global_search_stops_its_worker() {
    let fixture = tempfile::tempdir().expect("fixture");
    let roots = [fixture.path().join("home"), fixture.path().join("USB")];
    for root in &roots {
        fixture_file(root, "needle.txt");
    }
    let (search, events) = index_trees(roots.to_vec(), false);
    search.query("needle");
    drop(search);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        assert!(std::time::Instant::now() < deadline, "worker did not stop");
        match events.recv_timeout(Duration::from_millis(50)) {
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            _ => continue,
        }
    }
}
