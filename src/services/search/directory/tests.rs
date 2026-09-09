// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn directory_index_reports_budgets_and_unreadable_roots() {
    let fixture = tempfile::tempdir().expect("fixture");
    std::fs::write(fixture.path().join("one"), "fixture").expect("file");
    std::fs::write(fixture.path().join("two"), "fixture").expect("file");
    for (root, max_entries, time_budget, expected) in [
        (
            fixture.path().to_path_buf(),
            1,
            Duration::from_secs(10),
            SearchCoverage {
                entry_limit: true,
                ..Default::default()
            },
        ),
        (
            fixture.path().to_path_buf(),
            10,
            Duration::ZERO,
            SearchCoverage {
                time_limit: true,
                ..Default::default()
            },
        ),
        (
            fixture.path().join("missing"),
            10,
            Duration::from_secs(10),
            SearchCoverage {
                unreadable: true,
                ..Default::default()
            },
        ),
    ] {
        let index = SharedIndex::new();
        build_index(&index, vec![root], false, max_entries, time_budget);
        let data = index.state.read().expect("index data");
        assert!(!data.indexing);
        assert_eq!(data.coverage, expected);
        assert!(data.items.len() <= max_entries);
    }
}

#[test]
fn cancelled_directory_index_never_publishes_results() {
    let fixture = tempfile::tempdir().expect("fixture");
    std::fs::write(fixture.path().join("needle"), "fixture").expect("file");
    let index = SharedIndex::new();
    index.release();
    build_index(
        &index,
        vec![fixture.path().to_path_buf()],
        false,
        10,
        Duration::from_secs(10),
    );
    assert!(index.state.read().expect("index data").items.is_empty());
}
